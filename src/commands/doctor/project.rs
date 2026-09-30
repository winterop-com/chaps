//! Checks of the deployment directory: its files, name, volumes, `.env`, ports and components.

use super::*;

/// The files a deployment directory holds, as `chaps init` writes them: the
/// `.chaps/` state, `.env` and the compose files rendered from them.
///
/// The base stack is only one of them when chap-core is a component of this
/// deployment: `--without chap-core` renders neither `compose.yml` nor the
/// chaps-owned override, so looking for them would report a deployment that is
/// exactly as asked for as broken.
///
/// What a component keeps in its own directory
/// ([`crate::components::Component::dir`]) is deliberately not in here. Those
/// files are the operator's rather than rendered, and the `components` line
/// already judges each of them with the one thing to do about it - `chaps sync`
/// scaffolds a missing `dhis2/dhis.conf`, and it says why DHIS2 will not start
/// without it - so counting them here would report one fault on two lines and
/// offer `chaps init --force` for a file `sync` writes on its own.
/// [`files_verdict`] names the directories instead, so the count cannot be read
/// as covering them.
pub fn project_files(components: &Components) -> Vec<String> {
    let mut files = vec![
        format!("{CHAPS_DIR}/{PROJECT_FILE}"),
        format!("{CHAPS_DIR}/{MODELS_FILE}"),
        format!("{CHAPS_DIR}/{COMPONENTS_FILE}"),
        ENV_FILE.to_string(),
    ];
    if components.chap_core.enabled {
        files.push(BASE_COMPOSE.to_string());
        files.push(CHAPS_COMPOSE.to_string());
    }
    files.extend(components.compose_files());
    files.push(MARKETPLACE_COMPOSE.to_string());
    files
}

/// Whether every file a deployment is made of is still there.
///
/// `elsewhere` is the directories of the enabled components, which this check
/// does not look in. They are named on the `ok` line so the count is read for
/// what it is: `all 8 present` on a deployment whose `dhis2/dhis.conf` had been
/// deleted was a true count of a set that did not include the one file DHIS2
/// cannot start without, which is a line stating something untrue while the
/// `components` line right under it fails.
///
/// The `fail` carries none of it, for the reason [`ocs_part`]'s does not: the
/// one thing to do is the fix, and a deployment missing its rendered files is
/// told to render them again whatever else is also gone.
pub fn files_verdict(
    total: usize,
    missing: &[String],
    elsewhere: &[&str],
) -> (Status, String, Option<String>) {
    if missing.is_empty() {
        return (
            Status::Ok,
            format!("all {total} present{}", elsewhere_note(elsewhere)),
            None,
        );
    }
    (
        Status::Fail,
        format!("missing {}", missing.join(", ")),
        Some(
            "run `chaps sync` to render the compose files again, or `chaps init --force` here \
             to write the whole deployment"
                .to_string(),
        ),
    )
}

/// The tail that keeps `all N present` from claiming the component directories
/// it did not count: `; ocs/ and dhis2/ are on the `components` line`.
///
/// Empty for a deployment that has no such component, where there is nothing
/// left out and a tail would be noise.
fn elsewhere_note(dirs: &[&str]) -> String {
    let named: Vec<String> = dirs.iter().map(|dir| format!("{dir}/")).collect();
    let Some((last, rest)) = named.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        return format!("; {last} is on the `components` line");
    }
    format!(
        "; {} and {last} are on the `components` line",
        rest.join(", ")
    )
}

/// The `project-files` line for a directory on disk.
pub fn files_check(dir: &Path, components: &Components) -> Check {
    let wanted = project_files(components);
    let missing: Vec<String> = wanted
        .iter()
        .filter(|name| !dir.join(name).is_file())
        .cloned()
        .collect();
    // Only the enabled ones: the `components` line reports a component that is
    // on, so pointing at it for a directory left behind by one that is off would
    // send the reader to a line that says nothing about it.
    let elsewhere: Vec<&str> = Component::ALL
        .iter()
        .filter(|component| components.is_enabled(**component))
        .filter_map(|component| component.dir())
        .collect();
    Check::from_verdict(
        "project-files",
        "project files",
        files_verdict(wanted.len(), &missing, &elsewhere),
    )
}

/// The compose project name, and whether it is this deployment's own.
///
/// Compose names a project after its directory unless a file says otherwise,
/// so two deployments in directories both called `demo` share every container
/// name and every named volume - a fresh `chaps up` in the second one finds
/// the first one's database, with a password it has never seen. A deployment
/// written by this version of `chaps` records a name of its own; an older one
/// has the directory name and nothing else, which is what this warns about.
pub fn project_name_verdict(
    recorded: Option<&str>,
    dir_name: &str,
) -> (Status, String, Option<String>) {
    if let Some(name) = recorded {
        return (
            Status::Ok,
            format!("{name} (recorded in {CHAPS_DIR}/{PROJECT_FILE})"),
            None,
        );
    }
    let derived = crate::project::normalized_project_name(dir_name).unwrap_or_default();
    (
        Status::Warn,
        format!(
            "compose project name is the directory name; volumes can collide with other \
             deployments named {dir_name}"
        ),
        Some(format!(
            "run `chaps sync` to record it as `{derived}` in {CHAPS_DIR}/{PROJECT_FILE}; \
             it is the name compose already uses, so nothing is renamed"
        )),
    )
}

/// The `project` line.
pub fn project_check(project: &Project) -> Check {
    let dir_name = project
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Check::from_verdict(
        "compose-project",
        "project",
        project_name_verdict(project.compose_project(), &dir_name),
    )
}

/// Whether the named volumes this deployment would use are its own.
///
/// `created` is when `.chaps/project.yaml` was created and each volume's is
/// when docker created it. A database volume older than the deployment
/// directory itself was made by something else - an earlier deployment of the
/// same name, most often - and it still holds that deployment's role password,
/// which is exactly the state in which chap-core cannot log in.
///
/// Both times are optional: a filesystem that does not record a creation time,
/// and a docker that did not say, each cost the comparison and nothing else.
///
/// `ocs_data` is the OCS data volume and what it holds, when that could be
/// measured: the one volume under this prefix whose size is usually worth
/// knowing, because it is what a `chaps down --volumes` would destroy.
pub fn volume_verdict(
    prefix: &str,
    db_volume: &str,
    volumes: &[(String, Option<u64>)],
    created: Option<u64>,
    leftover: &[String],
    ocs_data: Option<(&str, u64)>,
) -> (Status, String, Option<String>) {
    if volumes.is_empty() {
        return (
            Status::Ok,
            format!("no {prefix}* volume yet; `chaps up` creates them"),
            None,
        );
    }
    let held = match ocs_data {
        Some((name, bytes)) => format!("; {name} holds {}", crate::backup::human_size(bytes)),
        None => String::new(),
    };
    let count = format!(
        "{} volume{} named {prefix}*{held}",
        volumes.len(),
        if volumes.len() == 1 { "" } else { "s" }
    );
    let older = volumes.iter().find(|(name, at)| {
        name == db_volume && matches!((at, created), (Some(a), Some(c)) if *a < c)
    });
    // The database one first: a deployment that cannot log in is a stack that
    // does not come up, where a volume nobody reads any more costs disk.
    if let Some((name, at)) = older {
        return (
            Status::Warn,
            format!(
                "the database volume {name} predates this deployment; if chap-core cannot log \
                 in, it belongs to an earlier deployment with the same name (volume {}, \
                 deployment {})",
                crate::backup::timestamp(at.unwrap_or_default()),
                crate::backup::timestamp(created.unwrap_or_default())
            ),
            Some(
                "remove it with `chaps down --volumes` if this deployment's data can go, \
                 or keep both by giving one of them a name of its own"
                    .to_string(),
            ),
        );
    }
    if !leftover.is_empty() {
        return (
            Status::Warn,
            format!(
                "leftover volumes from disabled models or components: {}{}",
                leftover.join(", "),
                // The OCS size only when that volume is one of them: after
                // this list, the size of one still in use reads as another
                // leftover.
                match ocs_data {
                    Some((name, _)) if leftover.iter().any(|l| l == name) => held.as_str(),
                    _ => "",
                }
            ),
            Some(LEFTOVER_FIX.to_string()),
        );
    }
    (Status::Ok, count, None)
}

/// What to do about the leftover volumes the check found.
///
/// `down --volumes` is not among the answers on purpose: it only removes the
/// volumes the compose files still declare, which is exactly the set these are
/// not in.
const LEFTOVER_FIX: &str = "remove each with `chaps models disable <id> --purge` or `chaps components disable <name> \
     --purge`, or `docker volume rm <name>`; keep them to have the data back when the model or \
     component is enabled again";

/// The volumes under this deployment's prefix that belong to nothing it still
/// enables.
///
/// A model's volume is `ck_<id>_data`, so a name of that shape whose id is no
/// longer in `.chaps/models.yaml` is a disabled model's data; the component
/// ones are named outright, and are leftovers while the component is off.
/// Every other name - the database, chap-core's own - belongs to the base
/// stack, which is nobody's leftover.
///
/// A component is asked whether the volume is among the ones it keeps, not
/// whether it is *the* one: a component with two volumes would otherwise have
/// the second reported as a leftover while the component is enabled, which is
/// an invitation to delete live data.
///
/// Pure, and injected with both lists: the verdict is decided here and the
/// docker call that gathers the names is [`volumes_check`]'s.
pub fn leftover_volumes(
    prefix: &str,
    volumes: &[String],
    models: &[String],
    components: &Components,
) -> Vec<String> {
    volumes
        .iter()
        .filter(|name| {
            let Some(bare) = name.strip_prefix(prefix) else {
                return false;
            };
            if let Some(id) = crate::compose::volume_model_id(bare) {
                return !models.iter().any(|enabled| enabled == id);
            }
            Component::ALL.iter().any(|component| {
                component.volumes().contains(&bare) && !components.is_enabled(*component)
            })
        })
        .cloned()
        .collect()
}

/// The `volumes` line: what docker holds under this deployment's name.
///
/// `running` decides whether the OCS data volume is measured here: while its
/// container is up, the `components` line has the size from inside it, and
/// starting a second container to measure what the first is writing to would
/// be both slower and less true. While it is down, this is the line that says
/// how much data a `chaps down --volumes` would destroy.
pub(super) fn volumes_check(project: &Project, running: &BTreeSet<String>) -> Check {
    const ID: &str = "volumes";
    const NAME: &str = "volumes";
    let Some(prefix) = project.volume_prefix() else {
        return Check::skip(ID, NAME, "this directory has no compose project name");
    };
    let volumes: Vec<(String, Option<u64>)> = docker::volumes_with_prefix(&prefix)
        .into_iter()
        .map(|v| (v.name, crate::status::parse_rfc3339(&v.created_at)))
        .collect();
    let created = deployment_created_at(project);
    let names: Vec<String> = volumes.iter().map(|(name, _)| name.clone()).collect();
    let models: Vec<String> = project.state.models.keys().cloned().collect();
    let leftover = leftover_volumes(&prefix, &names, &models, &project.state.components);
    let ocs_volume = format!("{prefix}{}", crate::compose::render::OCS_VOLUME);
    let ocs_data = (!running.contains(crate::compose::OCS_SERVICE) && names.contains(&ocs_volume))
        .then(|| docker::volume_size_bytes(&ocs_volume))
        .flatten()
        .map(|bytes| (ocs_volume.as_str(), bytes));
    Check::from_verdict(
        ID,
        NAME,
        volume_verdict(
            &prefix,
            &format!("{prefix}chap-db"),
            &volumes,
            created,
            &leftover,
            ocs_data,
        ),
    )
}

/// When this deployment directory was created, in Unix seconds.
///
/// The `.chaps/` directory rather than `project.yaml` inside it: every save
/// writes that file to a temporary sibling and renames it into place, so its
/// creation time is the time of the last `chaps sync` and not the time the
/// deployment was made. The directory is created once by `chaps init` and
/// never replaced.
///
/// `None` where the filesystem records no creation time, and the check that
/// uses it simply does not make the comparison.
fn deployment_created_at(project: &Project) -> Option<u64> {
    let chaps = project.chaps_dir();
    file_created_at(&chaps).or_else(|| file_created_at(&chaps.join(PROJECT_FILE)))
}

/// When a file or directory was created, in Unix seconds, when the filesystem
/// records it.
fn file_created_at(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.created())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// What `.env` says about the two things that bite later.
///
/// The default database password is the one a deployment reachable from
/// anywhere but the loopback should not keep, and a missing `CHAP_IMAGE_TAG`
/// line means the pin comments `sync` writes are gone. Whether authentication
/// is on is reported either way: "off" is a fact an operator has to be able to
/// see, not a fault.
///
/// `chaps_overlay` is the rendered `compose.chaps.yml`, which is where the
/// registration key is handed to chap-core: a protected deployment whose
/// override predates that line has a chap-core that answers every model's
/// registration with a 401.
pub fn env_verdict(
    body: Option<&str>,
    chaps_overlay: Option<&str>,
) -> (Status, String, Option<String>) {
    let Some(body) = body else {
        return (Status::Skip, "no .env to read".to_string(), None);
    };
    let auth_on = auth::active_value(body, auth::API_TOKEN_ENV_VAR).is_some();
    let password = auth::active_value(body, "POSTGRES_PASSWORD");
    let tag_line = body.lines().any(|line| {
        line.trim_start()
            .trim_start_matches('#')
            .trim_start()
            .starts_with(&format!("{CHAP_TAG_ENV_VAR}="))
    });

    let mut facts = vec![format!("auth {}", if auth_on { "on" } else { "off" })];
    let mut fixes = Vec::new();
    match password.as_deref() {
        None => {
            facts.push("POSTGRES_PASSWORD unset, so compose uses the default `chap`".to_string());
            fixes.push(PASSWORD_FIX);
        }
        Some("chap") => {
            facts.push("POSTGRES_PASSWORD is the default `chap`".to_string());
            fixes.push(PASSWORD_FIX);
        }
        Some(_) => facts.push("POSTGRES_PASSWORD set".to_string()),
    }
    if tag_line {
        facts.push(format!("{CHAP_TAG_ENV_VAR} present"));
    } else {
        facts.push(format!("no {CHAP_TAG_ENV_VAR} line"));
        fixes.push("run `chaps sync` to write the image pin comments back");
    }
    // Only a deployment with authentication on has anything to hand over, and
    // only an override rendered by an older chaps is missing the line.
    if auth_on
        && let Some(overlay) = chaps_overlay
        && !overlay.contains(auth::REGISTRATION_KEY_ENV_VAR)
    {
        facts.push(format!(
            "{CHAPS_COMPOSE} does not pass {} to chap-core",
            auth::REGISTRATION_KEY_ENV_VAR
        ));
        fixes.push(
            "run `chaps sync`, then `chaps restart`: without that line chap-core answers \
             every model registration with HTTP 401",
        );
    }

    let detail = facts.join(", ");
    if fixes.is_empty() {
        return (Status::Ok, detail, None);
    }
    (Status::Warn, detail, Some(fixes.join("; ")))
}

/// What to do about a database password anyone can guess.
const PASSWORD_FIX: &str = "set POSTGRES_PASSWORD in .env to a value of your own; a database volume that already \
     exists keeps the old password until `ALTER USER` changes it";

/// The `.env` line.
pub fn env_check(body: Option<&str>, chaps_overlay: Option<&str>) -> Check {
    Check::from_verdict("env", ".env", env_verdict(body, chaps_overlay))
}

/// Whether one host port the stack publishes is free to publish on.
///
/// `busy` is injected so the verdict can be tested without binding anything,
/// and the fix is the very sentence `chaps up`'s preflight would have failed
/// with, so the two never drift apart.
///
/// `note` is appended in parentheses to whatever the line says about the port:
/// [`api_port_note`] uses it to name the file that moved the API port, without
/// which the number looks wrong against `.chaps/project.yaml`.
pub fn port_check(
    claim: &PortClaim,
    running: &BTreeSet<String>,
    busy: &dyn Fn(u16) -> bool,
    suggestion: Option<u16>,
    note: Option<&str>,
) -> Check {
    let is_api = claim.service == API_SERVICE;
    let id = if is_api {
        "api-port".to_string()
    } else {
        format!("port-{}", claim.service)
    };
    let name = if is_api {
        "api port".to_string()
    } else {
        format!("port {}", claim.service)
    };
    let detail = |what: &str| match note {
        Some(note) => format!("{} {what} ({note})", claim.port),
        None => format!("{} {what}", claim.port),
    };
    if running.contains(&claim.service) {
        return Check::ok(id, name, detail("is held by this project's own container"));
    }
    if busy(claim.port) {
        return Check::fail(
            id,
            name,
            detail("is in use by something else"),
            ports::busy_line(claim, suggestion),
        );
    }
    Check::ok(id, name, detail("is free"))
}

/// What the `api port` line says about where its number came from, when that
/// is not what `.chaps/project.yaml` records.
///
/// compose reads `.env` last, so a `CHAP_API_PORT=` line there moves the
/// published port. The checklist has to name the file that did it: `18000 is
/// free` next to a recorded `api_port: 8000` otherwise reads as a bug in
/// `chaps` rather than as a deliberate override. The ordinary case, where the
/// two agree, says nothing.
pub fn api_port_note(project: &Project) -> Option<String> {
    let (port, source) = project.api_port_in_effect();
    (source == ApiPortSource::Env && port != project.state.api_port).then(|| {
        format!(
            "{}, over the {} recorded in .chaps/project.yaml",
            source.label(),
            project.state.api_port
        )
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn the_file_list_is_what_init_writes() {
        let files = project_files(&Components::default());
        assert_eq!(files.len(), 7);
        for name in [
            ".chaps/project.yaml",
            ".chaps/models.yaml",
            ".chaps/components.yaml",
            ".env",
            "compose.yml",
            "compose.chaps.yml",
            "compose.marketplace.yml",
        ] {
            assert!(files.contains(&name.to_string()), "{name} is missing");
        }

        // A component adds its own compose file, between the override and the
        // umbrella; chap-core off takes the base stack out of the list.
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);
        assert_eq!(
            project_files(&components),
            vec![
                ".chaps/project.yaml",
                ".chaps/models.yaml",
                ".chaps/components.yaml",
                ".env",
                "compose.yml",
                "compose.chaps.yml",
                "compose.ocs.yml",
                "compose.marketplace.yml",
            ]
        );
        components.set_enabled(crate::components::Component::ChapCore, false);
        assert_eq!(
            project_files(&components),
            vec![
                ".chaps/project.yaml",
                ".chaps/models.yaml",
                ".chaps/components.yaml",
                ".env",
                "compose.ocs.yml",
                "compose.marketplace.yml",
            ]
        );

        let (status, detail, fix) = files_verdict(6, &[], &[]);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "all 6 present");
        assert_eq!(fix, None);

        let (status, detail, fix) =
            files_verdict(6, &[".env".to_string(), "compose.yml".to_string()], &[]);
        assert_eq!(status, Status::Fail);
        assert_eq!(detail, "missing .env, compose.yml");
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync") && fix.contains("chaps init --force"));
    }

    /// The count is of the files this check looks for, and the line says so
    /// rather than reading as "nothing is missing from this deployment": a
    /// component's own directory is not in the list, and `dhis2/dhis.conf` is a
    /// file DHIS2 will not start without.
    #[test]
    fn the_files_line_names_the_component_directories_it_did_not_count() {
        assert_eq!(files_verdict(8, &[], &[]).1, "all 8 present");
        assert_eq!(
            files_verdict(8, &[], &["dhis2"]).1,
            "all 8 present; dhis2/ is on the `components` line"
        );
        assert_eq!(
            files_verdict(9, &[], &["ocs", "dhis2"]).1,
            "all 9 present; ocs/ and dhis2/ are on the `components` line"
        );
        assert_eq!(
            files_verdict(9, &[], &["ocs", "s3", "dhis2"]).1,
            "all 9 present; ocs/, s3/ and dhis2/ are on the `components` line"
        );
        // The fail has no room for it: the fix is the one thing to do.
        assert_eq!(
            files_verdict(8, &["compose.dhis2.yml".to_string()], &["dhis2"]).1,
            "missing compose.dhis2.yml"
        );
    }

    /// Which directories the line names: the enabled components' own, taken
    /// from [`Component::dir`] so a component added later is named without this
    /// check learning it.
    #[test]
    fn the_files_check_points_at_the_directories_of_the_enabled_components() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut components = Components::default();
        components.set_enabled(Component::Dhis2, true);
        for name in project_files(&components) {
            let path = root.join(&name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "written\n").unwrap();
        }

        // Every mandatory file is there and the line still says where the
        // DHIS2 config is judged, which is the whole of what was untrue before.
        let check = files_check(root, &components);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(
            check.detail, "all 8 present; dhis2/ is on the `components` line",
            "{check:?}"
        );

        // Whether the operator's file is there changes nothing on this line -
        // the `components` line is where that is judged - and the tail is what
        // sends the reader to it. It was absent for the assertion above.
        std::fs::create_dir_all(root.join(DHIS2_DIR)).unwrap();
        std::fs::write(root.join(DHIS2_DIR).join(DHIS2_CONFIG_FILE), "x\n").unwrap();
        assert_eq!(files_check(root, &components).detail, check.detail);

        // A deployment with no such component gets no tail.
        let plain = Components::default();
        for name in project_files(&plain) {
            let path = root.join(&name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "written\n").unwrap();
        }
        assert_eq!(files_check(root, &plain).detail, "all 7 present");
    }

    #[test]
    fn the_env_check_reads_the_two_things_that_bite_later() {
        // Nothing to read is not a fault: `project-files` already said so.
        assert_eq!(env_verdict(None, None).0, Status::Skip);

        let good = "POSTGRES_PASSWORD=0123456789abcdef\n\
                    CHAP_API_TOKEN=sekret\n\
                    # CHAP_IMAGE_TAG=latest\n";
        let (status, detail, fix) = env_verdict(Some(good), Some(OVERRIDE));
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "auth on, POSTGRES_PASSWORD set, CHAP_IMAGE_TAG present"
        );
        assert_eq!(fix, None);

        // Authentication off is reported, not complained about.
        let open = "POSTGRES_PASSWORD=0123456789abcdef\n# CHAP_API_TOKEN=\nCHAP_IMAGE_TAG=v2.3.1\n";
        let (status, detail, _) = env_verdict(Some(open), Some(OVERRIDE));
        assert_eq!(status, Status::Ok);
        assert!(detail.starts_with("auth off, "), "{detail}");

        // The default password is the one that has to be moved off.
        let (status, detail, fix) = env_verdict(
            Some("POSTGRES_PASSWORD=chap\n# CHAP_IMAGE_TAG=latest\n"),
            None,
        );
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("the default `chap`"), "{detail}");
        assert!(fix.unwrap().contains("ALTER USER"));

        // An unset one resolves to the same default through compose.
        let (status, detail, _) = env_verdict(Some("# CHAP_IMAGE_TAG=latest\n"), None);
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("unset"), "{detail}");

        // And a file the pin comments have been cut out of.
        let (status, detail, fix) = env_verdict(Some("POSTGRES_PASSWORD=0123456789abcdef\n"), None);
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("no CHAP_IMAGE_TAG line"), "{detail}");
        assert!(fix.unwrap().contains("chaps sync"));
    }

    /// A `compose.chaps.yml` as this version renders it: it hands chap-core
    /// the registration key.
    const OVERRIDE: &str = "services:\n  chap:\n    environment:\n      \
         SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}\n";

    #[test]
    fn a_protected_deployment_whose_override_drops_the_key_is_a_warning() {
        let protected = "POSTGRES_PASSWORD=0123456789abcdef\n\
                         CHAP_API_TOKEN=sekret\n\
                         # CHAP_IMAGE_TAG=latest\n";
        // What an older chaps rendered: the port override and nothing else.
        let old_override = "services:\n  chap:\n    ports: !override\n      - \"8000:8000\"\n";
        let (status, detail, fix) = env_verdict(Some(protected), Some(old_override));
        assert_eq!(status, Status::Warn);
        assert!(
            detail.contains("compose.chaps.yml does not pass SERVICEKIT_REGISTRATION_KEY"),
            "{detail}"
        );
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync"), "{fix}");
        assert!(fix.contains("401"), "{fix}");

        // The override this version renders says nothing.
        assert_eq!(env_verdict(Some(protected), Some(OVERRIDE)).0, Status::Ok);
        // Neither does a deployment with no authentication at all: there is
        // no key to hand over.
        let open = "POSTGRES_PASSWORD=0123456789abcdef\n# CHAP_IMAGE_TAG=latest\n";
        assert_eq!(env_verdict(Some(open), Some(old_override)).0, Status::Ok);
    }

    #[test]
    fn the_project_line_warns_while_the_name_is_only_the_directory() {
        // A deployment this version wrote records a name of its own.
        let (status, detail, fix) = project_name_verdict(Some("demo-1ab2c3"), "demo");
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "demo-1ab2c3 (recorded in .chaps/project.yaml)");
        assert_eq!(fix, None);

        // One written before that has the directory name, which another
        // deployment in another directory of the same name also has.
        let (status, detail, fix) = project_name_verdict(None, "demo");
        assert_eq!(status, Status::Warn);
        assert_eq!(
            detail,
            "compose project name is the directory name; volumes can collide with other \
             deployments named demo"
        );
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync"), "{fix}");
        // The fix renames nothing: it writes down the name compose already
        // uses, which is what keeps the running deployment's volumes.
        assert!(fix.contains("`demo`"), "{fix}");
        assert!(fix.contains("nothing is renamed"), "{fix}");
        assert!(
            project_name_verdict(None, "My Chap")
                .2
                .unwrap()
                .contains("`mychap`")
        );
    }

    /// When `.chaps/project.yaml` was created, in these tests.
    const CREATED: u64 = 1_790_147_400;

    #[test]
    fn a_database_volume_older_than_the_deployment_is_a_warning() {
        let prefix = "demo-1ab2c3_";
        let db = "demo-1ab2c3_chap-db";

        // Nothing started yet: nothing to be suspicious of.
        let (status, detail, _) = volume_verdict(prefix, db, &[], Some(CREATED), &[], None);
        assert_eq!(status, Status::Ok);
        assert!(detail.contains("no demo-1ab2c3_* volume yet"), "{detail}");

        // Volumes this deployment made itself.
        let mine = volumes(&[(db, CREATED + 60), ("demo-1ab2c3_logs", CREATED + 60)]);
        let (status, detail, fix) = volume_verdict(prefix, db, &mine, Some(CREATED), &[], None);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "2 volumes named demo-1ab2c3_*");
        assert_eq!(fix, None);

        // With a size for the OCS data volume, the line says how much data a
        // `down --volumes` would destroy rather than only how many volumes.
        let (status, detail, _) = volume_verdict(
            prefix,
            db,
            &mine,
            Some(CREATED),
            &[],
            Some(("demo-1ab2c3_ocs_data", 217_088 * 1024)),
        );
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "2 volumes named demo-1ab2c3_*; demo-1ab2c3_ocs_data holds 212.0 MB"
        );

        // A database volume that predates the directory it belongs to came
        // from somewhere else, and still holds that deployment's password.
        let inherited = volumes(&[(db, CREATED - 86_400), ("demo-1ab2c3_logs", CREATED + 60)]);
        let (status, detail, fix) =
            volume_verdict(prefix, db, &inherited, Some(CREATED), &[], None);
        assert_eq!(status, Status::Warn);
        assert!(
            detail.starts_with(
                "the database volume demo-1ab2c3_chap-db predates this deployment; \
                 if chap-core cannot log in, it belongs to an earlier deployment with \
                 the same name"
            ),
            "{detail}"
        );
        assert!(fix.unwrap().contains("chaps down --volumes"));

        // Neither time is guaranteed: a filesystem that records no creation
        // time, and a docker that did not say, each cost the comparison only.
        assert_eq!(
            volume_verdict(prefix, db, &inherited, None, &[], None).0,
            Status::Ok
        );
        let undated = vec![(db.to_string(), None)];
        assert_eq!(
            volume_verdict(prefix, db, &undated, Some(CREATED), &[], None).0,
            Status::Ok
        );
        // And an old volume that is not the database is not this warning.
        let other = volumes(&[("demo-1ab2c3_logs", CREATED - 86_400)]);
        assert_eq!(
            volume_verdict(prefix, db, &other, Some(CREATED), &[], None).0,
            Status::Ok
        );
    }

    /// The volume of a model or a component this deployment no longer enables
    /// is data nothing will ever mount again, and nothing else names it: the
    /// overlay that declared it is gone, so `down --volumes` cannot reach it
    /// either.
    #[test]
    fn a_volume_of_something_no_longer_enabled_is_a_warning_naming_it() {
        let prefix = "demo-1ab2c3_";
        let db = "demo-1ab2c3_chap-db";
        let ewars = "demo-1ab2c3_ck_chapkit_ewars_model_data";

        let held = volumes(&[(db, CREATED + 60), (ewars, CREATED + 60)]);
        let (status, detail, fix) = volume_verdict(
            prefix,
            db,
            &held,
            Some(CREATED),
            &[ewars.to_string(), "demo-1ab2c3_ocs_data".to_string()],
            None,
        );
        assert_eq!(status, Status::Warn);
        assert_eq!(
            detail,
            "leftover volumes from disabled models or components: \
             demo-1ab2c3_ck_chapkit_ewars_model_data, demo-1ab2c3_ocs_data"
        );
        // The OCS size stays off this line even when it was measured.
        let (_, detail, _) = volume_verdict(
            prefix,
            db,
            &volumes(&[(ewars, CREATED + 60)]),
            Some(CREATED),
            &[ewars.to_string()],
            Some(("demo-1ab2c3_ocs_data", 40 * 1024)),
        );
        assert!(!detail.contains("holds"), "{detail}");
        let fix = fix.expect("a leftover volume has something to do about it");
        assert!(fix.contains("chaps models disable <id> --purge"), "{fix}");
        assert!(
            fix.contains("chaps components disable <name> --purge"),
            "{fix}"
        );
        assert!(fix.contains("docker volume rm <name>"), "{fix}");
        // `down --volumes` is the one answer that does not work here.
        assert!(!fix.contains("--volumes"), "{fix}");

        // A database volume older than the deployment is the worse of the two
        // findings, and the one the line reports.
        let inherited = volumes(&[(db, CREATED - 86_400), (ewars, CREATED + 60)]);
        let (status, detail, _) = volume_verdict(
            prefix,
            db,
            &inherited,
            Some(CREATED),
            &[ewars.to_string()],
            None,
        );
        assert_eq!(status, Status::Warn);
        assert!(detail.starts_with("the database volume"), "{detail}");

        // A leftover OCS volume is measured too: it is data nothing will
        // mount again, and its size is what decides whether to keep it.
        let (_, detail, _) = volume_verdict(
            prefix,
            db,
            &held,
            Some(CREATED),
            &["demo-1ab2c3_ocs_data".to_string()],
            Some(("demo-1ab2c3_ocs_data", 3 * 1024 * 1024 * 1024)),
        );
        assert_eq!(
            detail,
            "leftover volumes from disabled models or components: \
             demo-1ab2c3_ocs_data; demo-1ab2c3_ocs_data holds 3.0 GB"
        );
    }

    /// Which of a deployment's volumes belong to nothing it still enables.
    #[test]
    fn the_leftovers_are_the_volumes_of_disabled_models_and_components() {
        let prefix = "demo-1ab2c3_";
        let names: Vec<String> = [
            "demo-1ab2c3_chap-db",
            "demo-1ab2c3_ck_chapkit_ewars_model_data",
            "demo-1ab2c3_ck_auto_arima_chapkit_data",
            "demo-1ab2c3_ocs_data",
            "demo-1ab2c3_s3_data",
            "otherdemo_ck_chapkit_ewars_model_data",
        ]
        .iter()
        .map(|n| n.to_string())
        .collect();

        // One model enabled, no component but chap-core: everything else
        // under this prefix is a leftover, and the other deployment's volume
        // is not this deployment's business.
        let models = vec!["chapkit_ewars_model".to_string()];
        let mut components = Components::default();
        assert_eq!(
            leftover_volumes(prefix, &names, &models, &components),
            [
                "demo-1ab2c3_ck_auto_arima_chapkit_data",
                "demo-1ab2c3_ocs_data",
                "demo-1ab2c3_s3_data"
            ]
        );

        // Turning the components on leaves only the disabled model's.
        components.set_enabled(Component::Ocs, true);
        components.set_enabled(Component::S3, true);
        assert_eq!(
            leftover_volumes(prefix, &names, &models, &components),
            ["demo-1ab2c3_ck_auto_arima_chapkit_data"]
        );

        // And with both models enabled there is nothing left over: the
        // database and chap-core's own volumes are the base stack's.
        let both = vec![
            "chapkit_ewars_model".to_string(),
            "auto_arima_chapkit".to_string(),
        ];
        assert!(leftover_volumes(prefix, &names, &both, &components).is_empty());
    }

    #[test]
    fn a_port_is_only_a_conflict_when_someone_else_holds_it() {
        let nothing = BTreeSet::new();
        let free = |_: u16| false;
        let taken = |_: u16| true;

        let api = claim(API_SERVICE, 8000);
        let check = port_check(&api, &nothing, &free, None, None);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.id, "api-port");
        assert_eq!(check.name, "api port");
        assert_eq!(check.detail, "8000 is free");

        let check = port_check(&api, &nothing, &taken, Some(8001), None);
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.detail, "8000 is in use by something else");
        assert_eq!(
            check.fix.unwrap(),
            ports::busy_line(&api, Some(8001)),
            "the fix is the sentence `chaps up` would have failed with"
        );

        // A port this project's own container publishes is ours, exactly as
        // the `up` preflight treats it.
        let running: BTreeSet<String> = [API_SERVICE.to_string()].into_iter().collect();
        let check = port_check(&api, &running, &taken, None, None);
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("this project's own container"));

        let model = claim("chapkit-ewars-model", 5001);
        let check = port_check(&model, &nothing, &taken, None, None);
        assert_eq!(check.id, "port-chapkit-ewars-model");
        assert_eq!(check.name, "port chapkit-ewars-model");
        assert!(check.fix.unwrap().contains("chaps models unexpose"));
    }

    /// `.env` moving the API port is the operator's doing, so the line names
    /// the file rather than looking like a number out of nowhere.
    #[test]
    fn the_api_port_line_names_the_file_that_moved_the_port() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: crate::project::ProjectState::default(),
        };
        project.state.api_port = 8000;

        // Nothing in .env: the recorded port stands, and there is nothing to
        // explain.
        assert_eq!(api_port_note(&project), None);
        // The line compose reads agrees with the recorded one: still nothing.
        std::fs::write(dir.path().join(ENV_FILE), "CHAP_API_PORT=8000\n").unwrap();
        assert_eq!(api_port_note(&project), None);

        std::fs::write(dir.path().join(ENV_FILE), "CHAP_API_PORT=18000\n").unwrap();
        let note = api_port_note(&project).expect("an override is worth a word");
        assert_eq!(
            note,
            "from .env, over the 8000 recorded in .chaps/project.yaml"
        );

        let claim = claim(API_SERVICE, project.effective_api_port());
        let check = port_check(&claim, &BTreeSet::new(), &|_| false, None, Some(&note));
        assert_eq!(
            check.detail,
            "18000 is free (from .env, over the 8000 recorded in .chaps/project.yaml)"
        );
    }
}
