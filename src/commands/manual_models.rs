//! `chaps models add|remove`: models the marketplace does not list.
//!
//! `add` resolves a repository URL or an image reference into the entry a
//! marketplace file would have carried, records it in
//! `.chaps/models-manual.yaml` and then enables it down the same path as any
//! other model. `remove` is the way back: it disables the model if it is on,
//! and then drops the definition.

use crate::cli::{ModelsAddArgs, ModelsRemoveArgs};
use crate::commands::Ctx;
use crate::compose::{ApplyReport, EnableRequest, Selection, apply};
use crate::error::{ChapError, Result};
use crate::manual::{self, AddRequest, Endpoints, Names, Origin, Resolved};
use crate::output::{self, Out};
use crate::project::{ManualModel, Project};
use crate::registry::Registry;
use serde::Serialize;

/// How `models add`'s own origin reads to every command downstream.
///
/// [`Origin::Default`] is the chapkit fallback rather than a table lookup, but
/// both are "this CLI's own last resort" and both are fixed the same way -
/// pass `--user`, or add the model where its image can be read.
fn user_source(origin: Origin) -> crate::compose::UserSource {
    use crate::compose::UserSource;
    match origin {
        Origin::Flag => UserSource::Flag,
        Origin::Image => UserSource::ImageConfig,
        Origin::Probe => UserSource::DockerProbe,
        Origin::Default => UserSource::Table,
    }
}

/// What `models add` did, and the shape of its `--json`.
#[derive(Debug, Serialize)]
struct AddReport {
    id: String,
    service_id: String,
    display_name: String,
    /// The repository URL or image reference the model was added from.
    source: String,
    image: String,
    /// The pin: a `sha-` tag, or `@sha256:...`.
    tag: String,
    /// Image and pin as docker is handed them.
    image_ref: String,
    commit: Option<String>,
    /// The branch `chaps update` follows, `null` for a pinned entry.
    follow: Option<String>,
    data_dir: String,
    user: String,
    runtime_amd64: bool,
    /// What the resolution had to guess at.
    notes: Vec<String>,
    #[serde(flatten)]
    apply: ApplyReport,
}

/// What `models remove` did.
#[derive(Debug, Serialize)]
struct RemoveReport {
    id: String,
    service_id: String,
    /// Whether the model was enabled, and therefore disabled on the way out.
    was_enabled: bool,
    #[serde(flatten)]
    apply: ApplyReport,
    /// Data volumes this run removed.
    purged: Vec<String>,
}

/// Add a model that is not in the marketplace, and enable it.
///
/// Nothing is written until the whole source has resolved: a repository with
/// no published build, an id that is already taken or a service name another
/// model holds all leave the deployment exactly as it was.
pub fn add(ctx: &Ctx, args: &ModelsAddArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let endpoints = Endpoints::from_env(ctx.registry.offline);
    let request = AddRequest {
        source: args.source.clone(),
        id: args.id.clone(),
        service_id: args.service_id.clone(),
        display_name: args.name.clone(),
        data_dir: args.data_dir.clone(),
        user: args.user.clone(),
        runtime_amd64: args.runtime_amd64,
    };
    // What it would be called is settled first, and checked against what the
    // deployment and the catalogue already hold: resolving the source pulls
    // an image for the uid probe, and nobody should wait for that only to be
    // told the id is taken. The marketplace is loaded on its own, so a
    // collision with it can be told from one with an earlier `models add`.
    let names = manual::names(&request)?;
    let mut registry = crate::registry::load(&ctx.registry)?;
    check_free(&project, &registry, &names)?;

    let resolved = manual::resolve(&request, &endpoints)?;

    let entry = ManualModel {
        service_id: resolved.service_id.clone(),
        display_name: resolved.display_name.clone(),
        repository: resolved.source.repository(),
        image: resolved.image.clone(),
        tag: resolved.tag.clone(),
        commit: resolved.commit.clone(),
        follow: resolved.follow.clone(),
        data_dir: Some(resolved.data_dir.clone()),
        user: Some(resolved.user.clone()),
        runtime_amd64: resolved.runtime_amd64,
        added: today(),
    };
    project
        .state
        .manual
        .insert(resolved.id.clone(), entry.clone());
    for warning in registry.with_manual(&project.state.manual) {
        output::warn(&warning);
    }

    let selection = Selection {
        enable: vec![EnableRequest {
            id: resolved.id.clone(),
            selector: resolved.selector(),
            port: args.port.map(|p| p.0),
            data_dir: Some(resolved.data_dir.clone()),
            user: Some(resolved.user.clone()),
            // `models add` has already asked the image; saying where its
            // answer came from keeps `models info` honest about an entry
            // nobody typed a `--user` for.
            user_from: Some(user_source(resolved.user_from)),
            allow_template: false,
            keep_version: false,
        }],
        disable: Vec::new(),
    };
    let applied = apply(&mut project, &registry, &selection, &endpoints)?;

    let mut notes = resolved.notes.clone();
    notes.push(format!(
        "the service must register with chap-core as `{}`; \
         if its own MLServiceInfo.id differs, `chaps status` shows it as unmanaged - \
         re-add it with `--service-id <that id>`",
        resolved.service_id
    ));
    let report = AddReport {
        id: resolved.id.clone(),
        service_id: resolved.service_id.clone(),
        display_name: resolved.display_name.clone(),
        source: resolved.source.describe(),
        image: resolved.image.clone(),
        tag: resolved.tag.clone(),
        image_ref: resolved.image_ref(),
        commit: resolved.commit.clone(),
        follow: resolved.follow.clone(),
        data_dir: resolved.data_dir.clone(),
        user: resolved.user.clone(),
        runtime_amd64: resolved.runtime_amd64,
        notes: notes.clone(),
        apply: applied,
    };
    ctx.out.emit(&report, || {
        format!(
            "{}{}",
            added_block(&resolved, &ctx.out),
            super::enable::summary(&report.apply, &notes, &project, &ctx.out)
        )
    })
}

/// Remove a manually added model: disable it if it is on, then drop the
/// definition.
///
/// A marketplace model is refused rather than half-removed: there is no local
/// definition to delete, and what the caller wants is `models disable`.
pub fn remove(ctx: &Ctx, args: &ModelsRemoveArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;
    let Some(id) = manual_id(&project, &args.id) else {
        return Err(match crate::registry::load(&ctx.registry)?.get(&args.id) {
            Some(model) => anyhow::anyhow!(
                "{} is a marketplace model, so there is no local definition to remove; \
                 run `chaps models disable {}`{}",
                model.id,
                model.id,
                if args.purge { " --purge" } else { "" }
            ),
            None => ChapError::UnknownModel(args.id.clone()).into(),
        });
    };

    let entry = project.state.manual[&id].clone();
    let was_enabled = project.state.models.contains_key(&id);
    let (mut applied, mut notes, mut purged) = (ApplyReport::default(), Vec::new(), Vec::new());
    if was_enabled {
        let (report, disable_notes) =
            super::enable::disable_enabled(&mut project, &registry, &id, args.purge)?;
        applied = report.apply;
        purged = report.purged;
        notes = disable_notes;
    } else if args.purge {
        match project.prefixed_volume(&crate::compose::volume_name(&id)) {
            Some(name) => {
                let (removed, line) = super::docker::purge_volume(&name);
                purged.extend(removed);
                notes.push(line);
            }
            None => notes.push(super::docker::UNNAMEABLE_VOLUME.to_string()),
        }
    }

    // The definition goes last: until it does, everything above still sees
    // the model the way the deployment recorded it.
    project.state.manual.remove(&id);
    project.save()?;

    let report = RemoveReport {
        id: id.clone(),
        service_id: entry.service_id.clone(),
        was_enabled,
        apply: applied,
        purged,
    };
    ctx.out.emit(&report, || {
        let mut text = format!(
            "{} {id} {}\n",
            ctx.out.warn("removed"),
            ctx.out
                .dim(&format!("({})", crate::project::MANUAL_MODELS_FILE))
        );
        if was_enabled {
            text.push_str(&super::enable::summary(
                &report.apply,
                &notes,
                &project,
                &ctx.out,
            ));
            return text;
        }
        for note in &notes {
            text.push_str(&format!(
                "{} {}\n",
                ctx.out.dim("note:"),
                ctx.out.backticks(note)
            ));
        }
        text.push_str(&ctx.out.dim("it was not enabled, so nothing else changed"));
        text
    })
}

/// The id a manually added model is recorded under, for whichever identifier
/// was typed: the id itself, or its compose service name.
fn manual_id(project: &Project, wanted: &str) -> Option<String> {
    if project.state.manual.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    project
        .state
        .manual
        .iter()
        .find(|(_, entry)| entry.service_id == wanted)
        .map(|(id, _)| id.clone())
}

/// Whether the id and the service name this add wants are free.
///
/// The marketplace is checked first because it is the one this deployment
/// does not control: an id it already lists is a model to enable, not one to
/// add, and a service name it holds would make two overlays fight over one
/// compose service.
fn check_free(project: &Project, marketplace: &Registry, names: &Names) -> Result<()> {
    let id = &names.id;
    if let Some(model) = marketplace.models.iter().find(|m| m.id == *id) {
        return Err(anyhow::anyhow!(
            "the marketplace already lists {id} ({}); run `chaps models enable {id}`, \
             or pass `--id <other>` to add this one beside it",
            model.display_name
        ));
    }
    if let Some(existing) = project.state.manual.get(id) {
        return Err(anyhow::anyhow!(
            "{id} was already added, from {}; run `chaps models remove {id}` first, \
             or pass `--id <other>`",
            existing
                .repository
                .clone()
                .unwrap_or_else(|| existing.image_ref())
        ));
    }
    let service_id = &names.service_id;
    if let Some(model) = marketplace
        .models
        .iter()
        .find(|m| m.service_id == *service_id)
    {
        return Err(anyhow::anyhow!(
            "the marketplace model {} already uses the compose service `{service_id}`; \
             pass `--service-id <other>`",
            model.id
        ));
    }
    if let Some((other, _)) = project
        .state
        .manual
        .iter()
        .find(|(_, entry)| entry.service_id == *service_id)
    {
        return Err(anyhow::anyhow!(
            "{other} already uses the compose service `{service_id}`; \
             pass `--service-id <other>`"
        ));
    }
    Ok(())
}

/// The block that says what was resolved, above the lines `enable` prints.
fn added_block(resolved: &Resolved, out: &Out) -> String {
    let pin = match &resolved.commit {
        Some(commit) => format!(
            "{}  {}",
            resolved.tag,
            out.dim(&format!("(commit {})", short(commit)))
        ),
        None => resolved.tag.clone(),
    };
    let follows = match &resolved.follow {
        Some(branch) => format!("{branch}  {}", out.dim("(`chaps update` moves the pin)")),
        None => format!("nothing {}", out.dim("(pinned)")),
    };
    format!(
        "{} {} {}\n{}",
        out.ok("added"),
        resolved.id,
        out.dim(&format!("({})", resolved.service_id)),
        output::fields_with(
            2,
            &[
                ("source", resolved.source.describe()),
                ("image", resolved.image_ref()),
                ("pin", pin),
                ("follows", follows),
                (
                    "data dir",
                    with_origin(&resolved.data_dir, resolved.data_dir_from, out)
                ),
                ("user", with_origin(&resolved.user, resolved.user_from, out)),
            ],
            &|label| out.dim(label),
        )
    )
}

/// `/work/data  (from the image config)`.
fn with_origin(value: &str, origin: Origin, out: &Out) -> String {
    format!("{value}  {}", out.dim(&format!("({})", origin.label())))
}

/// The first seven characters of a commit, as the image tag uses.
fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

/// Today in UTC, as `YYYY-MM-DD`.
fn today() -> String {
    crate::backup::timestamp(crate::backup::now())
        .chars()
        .take(10)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manual::source::Source;
    use crate::project::ProjectState;

    fn names_of(id: &str, service_id: &str) -> Names {
        Names {
            source: Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap(),
            id: id.to_string(),
            service_id: service_id.to_string(),
            display_name: "chapkit_ghr_model".into(),
        }
    }

    fn resolved(id: &str, service_id: &str) -> Resolved {
        Resolved {
            source: Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap(),
            id: id.to_string(),
            service_id: service_id.to_string(),
            display_name: "chapkit_ghr_model".into(),
            image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
            tag: "sha-b1d6c31".into(),
            commit: Some("b1d6c31f4b2f0d8a0f4c6e2a5e9d3c7b8a1f0e2d".into()),
            follow: Some("main".into()),
            data_dir: "/work/data".into(),
            data_dir_from: Origin::Image,
            user: "10001:10001".into(),
            user_from: Origin::Probe,
            runtime_amd64: true,
            notes: Vec::new(),
        }
    }

    fn project_with(manual: &[(&str, &str)]) -> Project {
        let mut state = ProjectState::default();
        for (id, service_id) in manual {
            state.manual.insert(
                id.to_string(),
                ManualModel {
                    service_id: service_id.to_string(),
                    display_name: id.to_string(),
                    repository: Some(format!("https://github.com/someone/{id}")),
                    image: format!("ghcr.io/someone/{id}"),
                    tag: "sha-0000000".into(),
                    commit: None,
                    follow: None,
                    data_dir: None,
                    user: None,
                    runtime_amd64: false,
                    added: "2026-09-24".into(),
                },
            );
        }
        Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state,
        }
    }

    #[test]
    fn an_id_the_marketplace_lists_is_a_model_to_enable() {
        let marketplace = crate::registry::load_embedded().unwrap();
        let project = project_with(&[]);
        let err = check_free(
            &project,
            &marketplace,
            &names_of("chapkit_ewars_model", "x-service"),
        )
        .expect_err("the marketplace lists it");
        assert!(err.to_string().contains("models enable"), "{err}");

        let err = check_free(
            &project,
            &marketplace,
            &names_of("something_else", "chapkit-ewars-model"),
        )
        .expect_err("the service name is taken");
        assert!(err.to_string().contains("--service-id"), "{err}");
    }

    #[test]
    fn an_id_this_deployment_added_is_removed_before_it_is_added_again() {
        let marketplace = crate::registry::load_embedded().unwrap();
        let project = project_with(&[("chapkit_ghr_model", "chapkit-ghr-model")]);

        let err = check_free(
            &project,
            &marketplace,
            &names_of("chapkit_ghr_model", "chapkit-ghr-model-2"),
        )
        .expect_err("already added");
        assert!(err.to_string().contains("models remove"), "{err}");

        let err = check_free(
            &project,
            &marketplace,
            &names_of("other_model", "chapkit-ghr-model"),
        )
        .expect_err("the service name is taken");
        assert!(err.to_string().contains("--service-id"), "{err}");

        // A free pair goes through.
        check_free(
            &project,
            &marketplace,
            &names_of("other_model", "other-model"),
        )
        .unwrap();
    }

    #[test]
    fn a_manual_model_answers_to_its_id_and_its_service_name() {
        let project = project_with(&[("chapkit_ghr_model", "chapkit-ghr-model")]);
        assert_eq!(
            manual_id(&project, "chapkit_ghr_model").as_deref(),
            Some("chapkit_ghr_model")
        );
        assert_eq!(
            manual_id(&project, "chapkit-ghr-model").as_deref(),
            Some("chapkit_ghr_model")
        );
        assert_eq!(manual_id(&project, "chapkit_ewars_model"), None);
    }

    #[test]
    fn the_added_block_says_where_every_value_came_from() {
        let out = Out::default();
        let text = added_block(&resolved("chapkit_ghr_model", "chapkit-ghr-model"), &out);
        assert!(text.starts_with("added chapkit_ghr_model (chapkit-ghr-model)\n"));
        assert!(
            text.contains("source    https://github.com/chap-models/chapkit_ghr_model"),
            "{text}"
        );
        assert!(
            text.contains("image     ghcr.io/chap-models/chapkit_ghr_model:sha-b1d6c31"),
            "{text}"
        );
        assert!(
            text.contains("pin       sha-b1d6c31  (commit b1d6c31)"),
            "{text}"
        );
        assert!(
            text.contains("follows   main  (`chaps update` moves the pin)"),
            "{text}"
        );
        assert!(
            text.contains("data dir  /work/data  (from the image config)"),
            "{text}"
        );
        assert!(
            text.contains("user      10001:10001  (from a docker probe)"),
            "{text}"
        );

        // A pinned entry says so where the branch would have been.
        let pinned = Resolved {
            follow: None,
            commit: None,
            ..resolved("chapkit_ghr_model", "chapkit-ghr-model")
        };
        let text = added_block(&pinned, &out);
        assert!(text.contains("follows   nothing (pinned)"), "{text}");
        assert!(text.contains("pin       sha-b1d6c31\n"), "{text}");
    }

    #[test]
    fn the_date_is_the_day_in_utc() {
        let date = today();
        assert_eq!(date.len(), 10, "{date}");
        assert_eq!(date.matches('-').count(), 2, "{date}");
        assert_eq!(short("b1d6c31f4b2f0d8a"), "b1d6c31");
    }
}
