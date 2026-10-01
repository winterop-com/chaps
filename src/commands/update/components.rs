//! The enabled components other than chap-core, which follow moving tags.

use crate::components;
use crate::docker;
use crate::project::Project;
use serde::Serialize;

/// One enabled component's image, and where its tag comes from.
///
/// Neither OCS nor the object store publishes a release feed this CLI can
/// consult, so both follow a moving tag: `docker compose pull` takes whatever
/// it points at today, and there is nothing in `.chaps/` to move. An active
/// `*_IMAGE_TAG` line in `.env` is the operator's own pin, and is reported as
/// such rather than silently overridden.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentUpdate {
    pub name: String,
    pub image: String,
    /// The tag the pull will take, whichever of the two it is.
    pub tag: String,
    /// Whether that tag came from an active `.env` line.
    pub pinned: bool,
}

/// One row per enabled component, read from `.chaps/components.yaml` and the
/// `.env` beside it.
pub fn plan_components(project: &Project) -> Vec<ComponentUpdate> {
    let env =
        std::fs::read_to_string(project.dir.join(crate::project::ENV_FILE)).unwrap_or_default();
    let components = &project.state.components;
    let mut out = Vec::new();
    if components.ocs.enabled {
        out.push(component_update(
            crate::compose::OCS_SERVICE,
            components::OCS_IMAGE,
            components::OCS_TAG_ENV_VAR,
            &components.ocs.image_tag,
            &env,
        ));
    }
    if components.s3.enabled {
        out.push(component_update(
            crate::compose::S3_SERVICE,
            components::S3_IMAGE,
            components::S3_TAG_ENV_VAR,
            components::S3_DEFAULT_TAG,
            &env,
        ));
    }
    if components.dhis2.enabled {
        out.push(component_update(
            crate::compose::DHIS2_SERVICE,
            &components.dhis2.image,
            components::DHIS2_TAG_ENV_VAR,
            &components.dhis2.image_tag,
            &env,
        ));
    }
    out
}

/// What a run that is about to re-pull the DHIS2 image is told while `dhis2_db`
/// is already there.
///
/// Not the line `chaps components enable dhis2` prints. That one is about a tag
/// an operator asked to change, and names the two tags. Here nothing on the
/// screen moves at all: `dhis2/core:2.42` is a minor line, so the same tag is a
/// newer patch release tomorrow, and a pin an operator set by hand is re-pulled
/// too. That is the whole reason the line exists - the pull is silent, the next
/// `chaps up` migrates the schema, and DHIS2 migrates forward only, so the only
/// way back from a migration that was not wanted is the archive taken before it.
pub fn dhis2_pull_warning(image: &str, tag: &str) -> String {
    format!(
        "this pull can bring a newer `{image}:{tag}`, and a newer DHIS2 migrates `dhis2_db` \
         irreversibly on the next `chaps up`: run `chaps backup create` first - an older image \
         on a migrated database answers healthy while every API request 404s"
    )
}

/// That warning, when this run has earned it: the component is enabled and its
/// database volume is already on this machine.
///
/// Best-effort about docker like every other step that needs it: no CLI and no
/// daemon both answer "no such volume", which is the quiet answer - a warning
/// about a database that may not exist would be worse than none.
pub(super) fn dhis2_pull_note(project: &Project, components: &[ComponentUpdate]) -> Option<String> {
    let row = components
        .iter()
        .find(|c| c.name == crate::compose::DHIS2_SERVICE)?;
    let volume = project.prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)?;
    if !docker::volume_exists(&volume) {
        return None;
    }
    Some(dhis2_pull_warning(&row.image, &row.tag))
}

fn component_update(
    name: &str,
    image: &str,
    var: &str,
    default_tag: &str,
    env: &str,
) -> ComponentUpdate {
    match crate::auth::active_value(env, var) {
        Some(tag) => ComponentUpdate {
            name: name.to_string(),
            image: image.to_string(),
            tag,
            pinned: true,
        },
        None => ComponentUpdate {
            name: name.to_string(),
            image: image.to_string(),
            tag: default_tag.to_string(),
            pinned: false,
        },
    }
}

/// The row one component gets in the human list.
pub fn component_line(c: &ComponentUpdate) -> String {
    if c.pinned {
        return format!(
            "{}  {}  pinned in .env, re-pulled at that tag",
            c.name, c.tag
        );
    }
    format!("{}  {}  moving tag, re-pulled", c.name, c.tag)
}
