//! What the configured models page asks chap-core, for the effects the
//! reducer records. Each call blocks, as the registry refresh does.

use crate::api::Api;
use crate::configs::{self, Draft};
use crate::error::ChapError;
use crate::project::Project;
use crate::registry::Registry;
use crate::tui::app::configs::Load;

/// The client for this deployment's chap-core.
fn api_of(project: &Project) -> Api {
    crate::commands::configs::api_of(project)
}

/// The line for a request that failed, with the way out.
fn failed(api: &Api, err: &anyhow::Error) -> String {
    match err.downcast_ref::<ChapError>() {
        Some(ChapError::Unreachable { .. }) => format!(
            "chap-core at {} is not running; start it with `varde up`, then press r",
            api.base()
        ),
        _ => err.to_string(),
    }
}

/// The configured models of `service` that are not archived, and its
/// template.
pub fn load(project: &Project, registry: &Registry, model: &str, service: &str) -> Load {
    let api = api_of(project);
    let listed = match configs::listing(&api) {
        Ok(listed) => listed,
        Err(err) => return Load::Failed(failed(&api, &err)),
    };
    let template = match configs::template_named(&api, service) {
        Ok(template) => template,
        Err(err) => return Load::Failed(failed(&api, &err)),
    };
    let source = crate::commands::configs::sync::source_of(registry, project, model);
    let configs = configs::configs_of(&listed, service)
        .into_iter()
        .filter(|config| !config.archived)
        .map(|config| {
            let label = configs::source_label(&config, Some(&source));
            (config, label)
        })
        .collect();
    Load::Ready { template, configs }
}

/// Create `draft`, which the user has confirmed: the footer line, and the
/// list again.
pub fn create(
    project: &Project,
    registry: &Registry,
    model: &str,
    service: &str,
    draft: &Draft,
) -> (String, Load) {
    let api = api_of(project);
    let made = configs::template_of_service(&api, model, service).and_then(|template| {
        let listed = configs::listing(&api).map_err(|err| failed(&api, &err))?;
        let existing = configs::configs_of(&listed, &template.name);
        configs::check(draft, &template, &existing, model)?;
        configs::create(&api, &template, draft).map_err(|err| failed(&api, &err))
    });
    let message = match made {
        Ok(_) => format!("created configured model {} of {model}", draft.variant),
        Err(why) => format!("could not create {}: {why}", draft.variant),
    };
    (message, load(project, registry, model, service))
}

/// Archive the configured model `id`, which the user has confirmed.
pub fn archive(
    project: &Project,
    registry: &Registry,
    model: &str,
    service: &str,
    id: i64,
    variant: &str,
) -> (String, Load) {
    let api = api_of(project);
    let message = match configs::archive(&api, id) {
        Ok(()) => format!("archived configured model {variant} of {model}"),
        Err(err) => format!("could not archive {variant}: {}", failed(&api, &err)),
    };
    (message, load(project, registry, model, service))
}
