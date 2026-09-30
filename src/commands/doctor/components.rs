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
/// which is a warning - it deploys, it just deploys Sierra Leone.
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
/// Modeling App cannot see CHAP finds the answer on the line they were already
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
mod tests {
    use super::super::test_support::*;
    use super::*;

    /// A deployment with `dhis2` on and nothing else but chap-core.
    fn dhis2_components() -> Components {
        let mut components = Components::default();
        components.set_enabled(Component::Dhis2, true);
        components
    }

    /// The DHIS2 half of the line: one file that has to be there, and one fact
    /// that is only ever reported.
    #[test]
    fn the_components_line_reports_the_dhis2_config_and_the_seed() {
        let components = dhis2_components();

        // The file is mandatory. Without it DHIS2 throws on startup, so this is
        // a fault rather than something to keep an eye on.
        let (status, detail, fix) =
            components_verdict(&components, &facts(None), &Dhis2Facts { config: false });
        assert_eq!(status, Status::Fail);
        assert_eq!(detail, "chap-core, dhis2; dhis2/dhis.conf is missing");
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync"), "{fix}");
        assert!(fix.contains("dhis2/dhis.conf"), "{fix}");

        // There, and the seed is the default the pinned minor line publishes.
        let (status, detail, fix) =
            components_verdict(&components, &facts(None), &Dhis2Facts { config: true });
        assert_eq!(status, Status::Ok, "the seed is never a fault: {detail}");
        assert_eq!(fix, None, "nothing here is something to do");
        assert_eq!(
            detail,
            format!(
                "chap-core, dhis2; dhis2/dhis.conf present; seed: default ({}); \
                 no `chaps dhis2 connect` recorded",
                crate::compose::render::DHIS2_DEFAULT_SEED_URL
            )
        );
    }

    /// The connect record is reported and never judged, and never as a state:
    /// the clause names the command and the time it ran, because that is the
    /// whole of what `.chaps/components.yaml` knows. `chaps dhis2 show` is the
    /// one thing that asks DHIS2.
    #[test]
    fn the_components_line_reports_the_recorded_connect_without_judging_it() {
        let present = Dhis2Facts { config: true };
        let mut components = dhis2_components();

        let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
        assert_eq!(status, Status::Ok, "a step left is not a fault: {detail}");
        assert_eq!(
            fix, None,
            "doctor cannot check the route, so it advises none"
        );
        assert!(
            detail.ends_with("; no `chaps dhis2 connect` recorded"),
            "{detail}"
        );

        components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
        let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
        assert_eq!(status, Status::Ok);
        assert_eq!(fix, None);
        assert!(
            detail.ends_with("; last `chaps dhis2 connect`: 2026-09-27T09:12:33Z"),
            "{detail}"
        );
        // Never "connected": nothing here asked DHIS2 anything.
        assert!(!detail.contains("connected:"), "{detail}");

        // A deployment with no chap-core has nothing to connect to, and
        // `chaps dhis2 connect` refuses there, so the clause is left off.
        components.set_enabled(Component::ChapCore, false);
        let (_, detail, _) = components_verdict(&components, &facts(None), &present);
        assert!(!detail.contains("chaps dhis2 connect"), "{detail}");
    }

    /// The three other answers the seed can have, each reported and none of
    /// them judged: an empty database is a deployment that brings its own data.
    #[test]
    fn the_seed_note_says_which_of_the_answers_this_deployment_holds() {
        let present = Dhis2Facts { config: true };
        // The clause that follows the seed on every `dhis2` line, tested on its
        // own above; naming it here keeps these assertions about the tail of
        // the seed note rather than about the end of the string.
        const NO_CONNECT: &str = "; no `chaps dhis2 connect` recorded";

        let mut components = dhis2_components();
        components.dhis2.seed = Dhis2Seed::None;
        let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
        assert_eq!(status, Status::Ok);
        assert_eq!(fix, None);
        assert!(
            detail.ends_with(&format!(
                "seed: none (the database starts empty){NO_CONNECT}"
            )),
            "{detail}"
        );

        // A dump of the operator's own, whether a URL or a path in the project.
        components.dhis2.seed = Dhis2Seed::parse("dumps/mine.sql.gz");
        let (_, detail, _) = components_verdict(&components, &facts(None), &present);
        assert!(
            detail.ends_with(&format!("seed: dumps/mine.sql.gz{NO_CONNECT}")),
            "{detail}"
        );

        // A minor line chaps publishes no dump for: the setting still says
        // `default`, and the database still starts empty, which is the half
        // that does not follow from the setting.
        components.dhis2.seed = Dhis2Seed::Default;
        components.dhis2.image_tag = "2.43".to_string();
        let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
        assert_eq!(status, Status::Ok);
        assert_eq!(fix, None);
        assert!(
            detail.ends_with(&format!(
                "seed: default, and chaps knows no dump for 2.43 \
                 (the database starts empty){NO_CONNECT}"
            )),
            "{detail}"
        );
    }

    /// The line carries every enabled component. An `ocs` with something to
    /// report used to be able to return before the rest of the line was
    /// written, which is the shape this must never take again.
    #[test]
    fn a_failing_ocs_does_not_swallow_what_the_dhis2_beside_it_says() {
        let mut components = dhis2_components();
        components.set_enabled(Component::Ocs, true);
        let example = crate::compose::render::render_ocs_config(
            &crate::compose::spec::OcsConfigSpec::default(),
        );

        // OCS is only warning, and the DHIS2 fault is the worse of the two, so
        // it is the one the line's status comes from - and both are printed.
        let (status, detail, fix) = components_verdict(
            &components,
            &facts(Some(&example)),
            &Dhis2Facts { config: false },
        );
        assert_eq!(status, Status::Fail);
        assert!(
            detail.contains("ocs/climate-service.yaml still holds"),
            "{detail}"
        );
        assert!(detail.contains("dhis2/dhis.conf is missing"), "{detail}");
        // One next step per fault, and both of them on the line.
        let fix = fix.unwrap();
        assert!(fix.contains("--ocs-country"), "{fix}");
        assert!(fix.contains("dhis2/dhis.conf"), "{fix}");

        // And the other way round: OCS's own config gone is the fault, and the
        // DHIS2 seed still rides on the line.
        let (status, detail, fix) =
            components_verdict(&components, &facts(None), &Dhis2Facts { config: true });
        assert_eq!(status, Status::Fail);
        assert!(
            detail.contains("ocs/climate-service.yaml is missing"),
            "{detail}"
        );
        assert!(
            detail.contains("dhis2/dhis.conf present; seed:"),
            "{detail}"
        );
        assert!(fix.unwrap().contains("chaps sync"));
    }

    /// Two things the line reports and never judges: missing credentials, which
    /// are optional, and the plugin count, which is a fact about a directory
    /// the operator put there.
    #[test]
    fn the_components_line_notes_the_credentials_and_the_plugins_without_complaining() {
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);

        let (status, detail, fix) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: false,
                plugins: Some(2),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
        assert_eq!(fix, None);
        assert!(
            detail
                .contains("ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)"),
            "{detail}"
        );
        assert!(detail.ends_with("plugins/: 2 files"), "{detail}");

        // One file is singular, and a set credential says nothing at all.
        let (_, detail, _) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                plugins: Some(1),
                ..OcsFacts::default()
            },
        );
        assert!(!detail.contains("credentials unset"), "{detail}");
        assert!(detail.ends_with("plugins/: 1 file"), "{detail}");
    }

    /// The tail rides on the example-config warning too. Enabling `ocs` leaves
    /// the example config behind by definition, so a tail that waited for the
    /// operator's own file would say nothing in exactly the run where
    /// ERA5-Land's missing key is the thing worth reading.
    #[test]
    fn the_example_config_warning_carries_the_same_tail_as_the_ok_line() {
        // The tail a deployment with no credentials and three plugin files
        // makes, spelled once for the two lines that have to carry the same one.
        const TAIL: &str = "; ERA5-Land: credentials unset (WorldPop and CHIRPS3 work \
                            without them); plugins/: 3 files";

        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);
        let example = crate::compose::render::render_ocs_config(
            &crate::compose::spec::OcsConfigSpec::default(),
        );

        // A freshly enabled OCS: the scaffolded config, no credentials in
        // `.env`, and a plugin directory the operator has started filling.
        let (status, detail, fix) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some(&example),
                credentials: false,
                plugins: Some(3),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Warn);
        assert_eq!(
            detail,
            format!(
                "chap-core, ocs; ocs/climate-service.yaml still holds OCS's example values{TAIL}"
            )
        );
        // One fault, one next step: the facts are on the status line and never
        // in the advice, because none of them is something to do.
        let fix = fix.unwrap();
        assert!(fix.contains("--ocs-country"), "{fix}");
        assert!(!fix.contains("credentials"), "{fix}");

        // Credentials set: the warning says nothing about them either way.
        let (status, detail, _) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some(&example),
                credentials: true,
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Warn);
        assert!(!detail.contains("credentials"), "{detail}");
        assert!(detail.ends_with("example values"), "{detail}");

        // The operator's own file: the same tail, after the same separator.
        let (status, edited, _) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: false,
                plugins: Some(3),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Ok);
        assert!(edited.ends_with(TAIL), "{edited}");

        // A missing config keeps none of it: nothing holds datasets or reads a
        // credential until the file is back, and `chaps sync` is the one step.
        let (status, detail, fix) = ocs_verdict(
            &components,
            &OcsFacts {
                config: None,
                credentials: false,
                plugins: Some(3),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Fail);
        assert!(detail.ends_with("is missing"), "{detail}");
        assert!(fix.unwrap().contains("chaps sync"));
    }

    /// The same two facts `chaps status` puts on the OCS line: what the
    /// instance holds, reported and never judged.
    #[test]
    fn the_components_line_reports_what_a_running_ocs_holds() {
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);

        let (status, detail, fix) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                datasets: Some(3),
                data_bytes: Some(217_088 * 1024),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
        assert_eq!(fix, None);
        assert_eq!(
            detail,
            "chap-core, ocs; ocs/climate-service.yaml present; 3 datasets; 212.0 MB data"
        );

        // One dataset is singular, and an instance that is not running, or
        // did not answer, says neither thing rather than saying nothing.
        let (_, detail, _) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                datasets: Some(1),
                ..OcsFacts::default()
            },
        );
        assert!(detail.ends_with("; 1 dataset"), "{detail}");
        let (_, detail, _) = ocs_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                ..OcsFacts::default()
            },
        );
        assert!(detail.ends_with("present"), "{detail}");
    }

    /// Files at any depth, because OCS's own layout is `plugins/datasets/*.py`.
    #[test]
    fn the_plugin_count_reaches_into_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        assert_eq!(plugin_count(&plugins), None, "no directory, no count");

        std::fs::create_dir_all(plugins.join("datasets")).unwrap();
        assert_eq!(plugin_count(&plugins), Some(0));
        std::fs::write(plugins.join("datasets").join("clms_gpp.py"), "").unwrap();
        std::fs::write(plugins.join("datasets").join("clms_gpp.yaml"), "").unwrap();
        std::fs::write(plugins.join("README.md"), "").unwrap();
        assert_eq!(plugin_count(&plugins), Some(3));
    }
}
