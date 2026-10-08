//! The compose project name a restore leaves behind, and the plan
//! `varde backup restore` prints before it asks.

use super::Manifest;
use serde::Serialize;
use std::path::PathBuf;

/// The compose project name a restore leaves the deployment with.
///
/// The destination keeps its own: the compose project name is what every
/// container and named volume of a deployment is prefixed with, so taking the
/// archive's would point this deployment at the volumes of the one the backup
/// came from - and leave its own behind, running, under no name anything still
/// refers to. Restoring into a second deployment is a normal thing to do (a
/// staging copy of production, a rebuild beside the original), and it must not
/// be a takeover.
///
/// `adopt` is `--adopt-identity`: the archive's name is taken over, which is
/// what a deployment restoring itself onto a new machine wants.
pub fn restored_compose_project(destination: &str, archived: &str, adopt: bool) -> String {
    if adopt {
        archived.trim().to_string()
    } else {
        destination.trim().to_string()
    }
}

/// The `compose_project` an archived `.varde/project.yaml` records, if any.
///
/// Read as plain YAML rather than through `ProjectState`, because this has to
/// work on a `project.yaml` this binary would refuse to deserialise.
pub fn archived_compose_project(body: &str) -> Option<String> {
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(body).ok()?;
    let name = value.get("compose_project")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// One model a restore will overwrite.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedModel {
    pub service_id: String,
    pub data_dir: String,
    pub volume: String,
}

/// One component data volume a restore will overwrite.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedComponent {
    pub name: String,
    pub service: String,
    pub data_dir: String,
    pub volume: String,
    /// Whether the archive holds a `pg_dump` of this database rather than a
    /// tar of its volume. See [`super::is_dhis2_db_dump`].
    pub dump: bool,
}

/// What `varde backup restore` is about to do, printed before it asks.
#[derive(Debug, Clone, Serialize)]
pub struct RestorePlan {
    pub archive: PathBuf,
    pub project_dir: PathBuf,
    pub manifest: Manifest,
    /// Project files that will be written, relative to the project directory.
    pub files: Vec<String>,
    /// Whether the database will be restored over the running one.
    pub database: bool,
    /// Model data volumes that will be emptied and refilled.
    pub models: Vec<PlannedModel>,
    /// Component data volumes that will be emptied and refilled.
    pub components: Vec<PlannedComponent>,
    /// Running services that will be stopped first.
    pub stop: Vec<String>,
    /// Whether the services are started at the end.
    pub start: bool,
    /// Whether `--files-only` was passed: no service is asked about, stopped
    /// or started.
    pub files_only: bool,
    /// The compose project name the deployment is left with. See
    /// [`restored_compose_project`].
    pub compose_project: String,
    /// The compose project name the archive was taken under, when it recorded
    /// one.
    pub archived_compose_project: Option<String>,
    /// Whether `--adopt-identity` was passed.
    pub adopt_identity: bool,
    /// The other directories that record the compose project name this
    /// restore takes over: they share its containers and volumes.
    pub shared_with: Vec<PathBuf>,
}

impl RestorePlan {
    /// Whether the plan would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && !self.database
            && self.models.is_empty()
            && self.components.is_empty()
    }
}

/// What the plan says about the compose project name, when there is anything
/// to say: nothing when the archive records none, and nothing when it records
/// the one this deployment already has.
///
/// Worth a line of its own whenever the two differ, because it is the one
/// thing a restore deliberately does not take from the archive. See
/// [`restored_compose_project`].
pub fn identity_line(plan: &RestorePlan) -> String {
    let Some(archived) = plan.archived_compose_project.as_deref() else {
        return String::new();
    };
    let kept = &plan.compose_project;
    if plan.adopt_identity {
        return format!(
            "  identity    compose project {archived}, taken over from the archive \
             (--adopt-identity)\n"
        );
    }
    if archived == plan.compose_project {
        return String::new();
    }
    format!(
        "  identity    {kept} is kept; the archive's own ({archived}) is not adopted, so this \
         deployment keeps its containers and volumes\n"
    )
}

/// The plan as an operator reads it before answering the confirmation.
pub fn plan_text(plan: &RestorePlan) -> String {
    let manifest = &plan.manifest;
    let mut text = String::new();
    text.push_str(&format!("restore {}\n", plan.archive.display()));
    text.push_str(&crate::output::fields(
        2,
        &[
            (
                "taken",
                format!("{} by {}", manifest.created_at, manifest.created_by),
            ),
            (
                "from",
                format!(
                    "deployment {} (chap-core {})",
                    manifest.project, manifest.chap_image_tag
                ),
            ),
            ("into", plan.project_dir.display().to_string()),
        ],
    ));

    text.push_str("\nthis overwrites\n");
    if plan.files.is_empty() {
        text.push_str("  files       nothing\n");
    } else {
        text.push_str(&format!(
            "  files       {} file(s) in the deployment directory: {}\n",
            plan.files.len(),
            plan.files.join(", ")
        ));
    }
    match (&plan.database, &manifest.database) {
        (true, Some(db)) => text.push_str(&format!(
            "  database    {} on postgres, dropped and reloaded (pg_restore --clean --if-exists)\n",
            db.name
        )),
        _ => text.push_str("  database    nothing\n"),
    }
    if plan.models.is_empty() {
        text.push_str("  models      nothing\n");
    } else {
        for (i, model) in plan.models.iter().enumerate() {
            let label = if i == 0 {
                "  models    "
            } else {
                "            "
            };
            text.push_str(&format!(
                "{label}  {} {} emptied and refilled (volume {})\n",
                model.service_id, model.data_dir, model.volume
            ));
        }
    }
    if plan.components.is_empty() {
        text.push_str("  components  nothing\n");
    } else {
        for (i, part) in plan.components.iter().enumerate() {
            let label = if i == 0 {
                "  components"
            } else {
                "            "
            };
            text.push_str(&format!("{label}  {}\n", component_line(part)));
        }
    }
    text.push_str(&identity_line(plan));
    for other in &plan.shared_with {
        text.push_str(&format!(
            "  shared      {}\n",
            crate::known::shared_name_line(&plan.compose_project, other)
        ));
    }

    text.push('\n');
    text.push_str(&services_text(plan));
    text
}

/// What the plan says a restore does to one component volume.
fn component_line(part: &PlannedComponent) -> String {
    match part.dump {
        true => format!(
            "{} database dropped and reloaded from the pg_dump (pg_restore -j {})",
            part.service,
            super::DHIS2_RESTORE_JOBS
        ),
        false => format!(
            "{} {} emptied and refilled (volume {})",
            part.service, part.data_dir, part.volume
        ),
    }
}

/// What the plan says about the services: which are stopped first, and
/// whether they are started at the end.
fn services_text(plan: &RestorePlan) -> String {
    // Files only: nothing asked docker what runs, so nothing is said about it.
    if plan.files_only {
        return "files only (--files-only): no service is stopped or started\n".to_string();
    }
    let mut text = String::new();
    if plan.stop.is_empty() {
        text.push_str("nothing is running, so nothing is stopped first\n");
    } else {
        text.push_str(&format!("stops first  {}\n", plan.stop.join(", ")));
    }
    if plan.start {
        text.push_str("then runs    the port check of `varde up`, then docker compose up -d\n");
    } else {
        text.push_str("then leaves  the services as they are (--no-start)\n");
    }
    text
}
