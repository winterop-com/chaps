//! The components line: which components are enabled, and what OCS and DHIS2 need.

use super::*;

/// What the `components` line says about the enabled set and the files that go
/// with it.
///
/// Everything about the `ocs` component that is on disk rather than in
/// `components.yaml`, gathered once so the verdict is a pure function of it.
#[derive(Debug, Clone, Default)]
pub struct OcsFacts<'a> {
    /// The body of `ocs/climate-service.yaml`, or `None` when there is none.
    pub config: Option<&'a str>,
    /// Whether `.env` sets any of the dataset credential variables.
    pub credentials: bool,
    /// How many files `ocs/plugins/` holds, or `None` when the project has no
    /// plugin directory at all.
    pub plugins: Option<usize>,
    /// How many datasets the running instance holds, from its JSON API, or
    /// `None` when it was not running or did not answer.
    pub datasets: Option<u32>,
    /// How much its data directory holds, in bytes, read from inside the
    /// running container. `None` while it is not running: what the volume
    /// holds is the `volumes` line's to report, where the question is what a
    /// `down --volumes` would destroy.
    pub data_bytes: Option<u64>,
}

/// Everything about the `dhis2` component that is on disk rather than in
/// `components.yaml`. The seed is not here: it is recorded state, so it is read
/// off [`Components`] itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Dhis2Facts {
    /// Whether `dhis2/dhis.conf` is there. Its absence is fatal: DHIS2 throws
    /// on startup without it, and there is no environment-only mode.
    pub config: bool,
}

/// What one component contributes to the `components` line: the status it
/// forces, the text that goes after the enabled set, and the one thing to do
/// about it.
///
/// A value rather than an early return, and deliberately so. The line carries
/// every enabled component, so an `ocs` whose config file has gone must not be
/// able to swallow what the `dhis2` beside it had to say - which is the same
/// mistake the OCS tail itself was fixed for, one level down, when the warning
/// arm returned before printing it.
struct ComponentPart {
    status: Status,
    detail: String,
    fix: Option<String>,
}

/// How bad one status is, for keeping the worst of several parts.
fn severity(status: Status) -> u8 {
    match status {
        Status::Ok => 0,
        Status::Skip => 1,
        Status::Warn => 2,
        Status::Fail => 3,
    }
}

/// The enabled set, plus what each enabled component's own files say.
///
/// Every part is appended, and the line's status is the worst of them: two
/// components can each have something to report, and either one going quiet
/// because the other is worse is the one outcome this must not have.
pub fn components_verdict(
    components: &Components,
    ocs: &OcsFacts,
    dhis2: &Dhis2Facts,
) -> (Status, String, Option<String>) {
    let mut detail = components.label();
    let mut status = Status::Ok;
    let mut fixes: Vec<String> = Vec::new();
    for part in [ocs_part(components, ocs), dhis2_part(components, dhis2)]
        .into_iter()
        .flatten()
    {
        detail.push_str("; ");
        detail.push_str(&part.detail);
        if severity(part.status) > severity(status) {
            status = part.status;
        }
        fixes.extend(part.fix);
    }
    let fix = (!fixes.is_empty()).then(|| fixes.join("; "));
    (status, detail, fix)
}

/// `ocs.config` is what is on disk at `ocs/climate-service.yaml`: `None` when
/// there is none, which is a real fault (the container would start with no
/// instance configuration), and a body that still carries the example marker,
/// which is a warning - it deploys, it just deploys Laos.
///
/// The credentials and the plugin count are only ever reported, never judged. A
/// deployment with no dataset credentials is a working deployment: WorldPop and
/// CHIRPS3 need none, and an operator who only wants those should not be told
/// once a day that something is unset on purpose.
///
/// The tail those facts make ([`ocs_notes`]) rides on the warning as well, in
/// the same place it sits on the `ok` line. A deployment that has just enabled
/// `ocs` still has the example config by definition, and that is exactly the
/// run in which the operator most needs to read that ERA5-Land has no key yet;
/// a tail that waited for the file to be edited would be silent when it is
/// worth most. The advice line stays the one thing to do, because none of the
/// facts is something to do.
///
/// The missing-config `fail` carries none of it: there is no instance to hold
/// datasets, credentials or plugins until the file is back, `chaps sync` is the
/// one next step either way, and the dataset count and the data size are only
/// ever read out of a running container, which a deployment in that state does
/// not have.
fn ocs_part(components: &Components, facts: &OcsFacts) -> Option<ComponentPart> {
    if !components.ocs.enabled {
        return None;
    }
    let config = format!("{OCS_DIR}/{OCS_CONFIG_FILE}");
    let Some(body) = facts.config else {
        return Some(ComponentPart {
            status: Status::Fail,
            detail: format!("{config} is missing"),
            fix: Some(
                "run `chaps sync` to scaffold it again, then edit it for your country".to_string(),
            ),
        });
    };
    if body.contains(OCS_EXAMPLE_MARKER) {
        return Some(ComponentPart {
            status: Status::Warn,
            detail: format!(
                "{config} still holds OCS's example values{}",
                ocs_notes(facts)
            ),
            fix: Some(format!(
                "edit {config} for your own country or region and delete the note at the top; \
                 `chaps components enable ocs --ocs-name NAME --ocs-country CODE --ocs-bbox \
                 xmin,ymin,xmax,ymax` writes it for a project that has none"
            )),
        });
    }
    Some(ComponentPart {
        status: Status::Ok,
        detail: format!("{config} present{}", ocs_notes(facts)),
        fix: None,
    })
}

/// The `dhis2` half: whether its one mandatory file is there, what the database
/// will be seeded from, and whether a `chaps dhis2 connect` has been recorded.
///
/// The missing file is the only thing here that is judged, and it is a fault
/// with nothing arguable about it - DHIS2 throws on startup without
/// `dhis2/dhis.conf` and there is no environment-only mode, so the container
/// would come up and stop. The seed is the OCS credentials line's counterpart:
/// reported, never judged. An empty database is a deployment that brings its own
/// data, not a deployment that is wrong.
///
/// The connect record is reported on exactly those terms, and for the same
/// reason it is never judged: a deployment nothing has connected is one with a
/// step left, not a broken one, and `chaps doctor` holds no DHIS2 credentials
/// and could not check the route if it wanted to. What it can do is say what
/// `.chaps/components.yaml` holds, in words that cannot be read as a verdict on
/// the route - the command and the time it ran - so that an operator whose
/// Modeling App cannot see Chap finds the answer on the line they were already
/// reading. [`crate::commands::status`] is where the same fact turns into a
/// hint, and only there, because that is the command that has asked DHIS2
/// whether it is even answering.
fn dhis2_part(components: &Components, facts: &Dhis2Facts) -> Option<ComponentPart> {
    if !components.dhis2.enabled {
        return None;
    }
    let config = format!("{DHIS2_DIR}/{DHIS2_CONFIG_FILE}");
    if !facts.config {
        return Some(ComponentPart {
            status: Status::Fail,
            detail: format!("{config} is missing"),
            fix: Some(format!(
                "run `chaps sync` to scaffold {config} again: DHIS2 throws on startup without it, \
                 and there is no environment-only mode"
            )),
        });
    }
    Some(ComponentPart {
        status: Status::Ok,
        detail: format!(
            "{config} present; {}{}",
            dhis2_seed_note(components),
            dhis2_connect_note(components)
        ),
        fix: None,
    })
}

/// What the `dhis2` part says about the recorded connect: the time
/// `chaps dhis2 connect` last finished here, or that nothing has.
///
/// Named as the command and a timestamp rather than as a state, because that is
/// all it is. "connected: yes" would be a claim about a DHIS2 nothing here
/// asked; "last `chaps dhis2 connect`: the time it ran" is a fact about this
/// file.
///
/// Empty on a deployment with no chap-core, where there is nothing to connect
/// to and `chaps dhis2 connect` refuses - the same line
/// [`Components::dhis2_needs_connecting`] draws.
fn dhis2_connect_note(components: &Components) -> String {
    if !components.chap_core.enabled {
        return String::new();
    }
    match &components.dhis2.connected_at {
        Some(at) => format!("; last `chaps dhis2 connect`: {at}"),
        None => "; no `chaps dhis2 connect` recorded".to_string(),
    }
}

/// What the `dhis2` part says about the seed: which of the three answers
/// `.chaps/components.yaml` holds, and what `default` resolves to.
///
/// The resolution is worth printing because it is the half that does not follow
/// from the setting: a minor line chaps publishes no dump for resolves to
/// nothing, and `chaps doctor` is the last place to learn that before the first
/// `chaps up` brings up an empty DHIS2 with no explanation.
fn dhis2_seed_note(components: &Components) -> String {
    if components.dhis2_seed_is_unknown() {
        return format!(
            "seed: default, and chaps knows no dump for {} (the database starts empty)",
            crate::components::dhis2_minor(&components.dhis2.image_tag)
        );
    }
    match components.dhis2_seed_source() {
        None => "seed: none (the database starts empty)".to_string(),
        Some(source) if components.dhis2.seed == Dhis2Seed::Default => {
            format!("seed: default ({source})")
        }
        Some(source) => format!("seed: {source}"),
    }
}

/// The informational tail of the `components` line: what the OCS instance has
/// beyond its config file. The same tail whether that file is the operator's
/// own or still the example, so a reader who has seen one line recognises the
/// other and `--json` carries the facts in `detail` either way.
fn ocs_notes(facts: &OcsFacts) -> String {
    let mut notes = String::new();
    if !facts.credentials {
        notes.push_str("; ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)");
    }
    if let Some(count) = facts.plugins {
        notes.push_str(&format!(
            "; {OCS_PLUGINS_DIR}/: {count} file{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(count) = facts.datasets {
        notes.push_str(&format!(
            "; {count} dataset{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(bytes) = facts.data_bytes {
        notes.push_str(&format!("; {} data", crate::backup::human_size(bytes)));
    }
    notes
}

/// The `components` line for a project on disk.
///
/// `running` is the set of compose services that are up, which is what decides
/// whether the instance is asked what it holds: a dataset count and a data
/// size are read out of a container that is there, and neither is worth a
/// timeout spent on one that is not.
pub fn components_check(project: &Project, running: &BTreeSet<String>) -> Check {
    let body = std::fs::read_to_string(project.ocs_config_path()).ok();
    let env = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let live =
        project.state.components.ocs.enabled && running.contains(crate::compose::OCS_SERVICE);
    let facts = OcsFacts {
        config: body.as_deref(),
        credentials: crate::components::OCS_DATA_SOURCE_ENV_VARS
            .iter()
            .any(|var| crate::dotenv::non_empty(&env, var).is_some()),
        plugins: plugin_count(&project.ocs_plugins_path()),
        datasets: live
            .then(|| project.state.components.ocs_url())
            .flatten()
            .and_then(|url| crate::status::ocs_datasets(&url)),
        data_bytes: live.then(|| docker::ocs_data_bytes(project)).flatten(),
    };
    // Nothing is asked of a running DHIS2 here. What `chaps status` can learn
    // over HTTP it has already put on the `dhis2` line, and everything past that
    // - the version most of all - is behind a login this CLI does not hold.
    let dhis2 = Dhis2Facts {
        config: sync::dhis2_config_path(&project.dir).is_file(),
    };
    Check::from_verdict(
        "components",
        "components",
        components_verdict(&project.state.components, &facts, &dhis2),
    )
}

/// How many files the plugin directory holds, or `None` when there is none.
///
/// Files at any depth, because OCS's own layout is `plugins/datasets/*.py`: a
/// count of the top level would read `1` for a directory with a dozen datasets
/// in it.
fn plugin_count(dir: &Path) -> Option<usize> {
    if !dir.is_dir() {
        return None;
    }
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
        }
    }
    Some(count)
}

#[cfg(test)]
mod tests;
