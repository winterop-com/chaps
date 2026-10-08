use super::clock::{clock_of, tzif_offset};
use super::*;
use std::time::Duration;

#[test]
fn a_traced_body_never_carries_a_bearer_token() {
    let body = r#"{"auth":{"type":"api-headers","headers":{"Authorization":"Bearer s3cret"}}}"#;
    let traced = trace_body(body);
    assert!(!traced.contains("s3cret"), "{traced}");
    assert!(
        traced.contains(r#""Authorization":"Bearer <redacted>""#),
        "{traced}"
    );
    assert_eq!(trace_body("no token here"), "no token here");
    assert_eq!(
        trace_body("Bearer a Bearer b"),
        "Bearer <redacted> Bearer <redacted>"
    );
}

/// A terminal that wants colour.
fn colored() -> Out {
    Out {
        color: true,
        tty: true,
        ..Out::default()
    }
}

#[test]
fn table_pads_columns_and_trims_line_ends() {
    let out = Out::default();
    let text = out.table(
        &["ID", "PORT"],
        &[
            vec!["chapkit_ewars_model".into(), "5001".into()],
            vec!["auto_arima_chapkit".into(), "5002".into()],
        ],
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "ID                   PORT");
    assert_eq!(lines[1], "chapkit_ewars_model  5001");
    assert_eq!(lines[2], "auto_arima_chapkit   5002");
    assert!(lines.iter().all(|l| !l.ends_with(' ')));
    assert!(text.ends_with('\n'));

    // A row whose last cells are empty ends after its last word, not in
    // the padding of the columns before them.
    let text = out.table(
        &["VERSION", "STATUS", "CHAPKIT", "CHANGELOG"],
        &[vec![
            "sha-b1d6c31".into(),
            "unstable".into(),
            String::new(),
            String::new(),
        ]],
    );
    assert_eq!(text.lines().nth(1), Some("sha-b1d6c31  unstable"));
}

#[test]
fn table_widens_a_column_to_fit_its_header() {
    let out = Out::default();
    let text = out.table(
        &["ID", "ASSESSED STATUS", "PORT"],
        &[vec!["a".into(), "green".into(), "5001".into()]],
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "ID  ASSESSED STATUS  PORT");
    assert_eq!(lines[1], "a   green            5001");
}

#[test]
fn table_tolerates_a_short_row() {
    let out = Out::default();
    let text = out.table(&["A", "B"], &[vec!["only".into()]]);
    assert_eq!(text.lines().nth(1), Some("only"));
}

#[test]
fn table_of_nothing_is_empty() {
    assert_eq!(Out::default().table(&[], &[]), "");
}

#[test]
fn a_styled_cell_occupies_the_width_of_its_text() {
    let out = colored();
    let text = out.table(
        &["ID", "STATUS", "PORT"],
        &[
            vec!["a".into(), out.ok("green"), "5001".into()],
            vec!["bbbb".into(), out.bad("red"), "5002".into()],
        ],
    );
    // Row 0 is the header, row 1 the rule colour adds, then the body.
    let rows: Vec<String> = text
        .lines()
        .skip(2)
        .map(|l| console::strip_ansi_codes(l).to_string())
        .collect();
    assert_eq!(rows[0], "a     green   5001");
    assert_eq!(rows[1], "bbbb  red     5002");
}

#[test]
fn colour_adds_a_rule_under_the_header_and_plain_does_not() {
    let rows = vec![vec!["a".into(), "5001".into()]];
    assert!(colored().table(&["ID", "PORT"], &rows).contains('─'));
    assert!(
        !Out {
            tty: true,
            ..Out::default()
        }
        .table(&["ID", "PORT"], &rows)
        .contains('─')
    );
    assert!(!Out::default().table(&["ID", "PORT"], &rows).contains('─'));
}

#[test]
fn styling_helpers_are_a_no_op_without_colour() {
    let out = Out::default();
    assert_eq!(out.heading("Next"), "Next");
    assert_eq!(out.ok("up"), "up");
    assert_eq!(out.warn("slow"), "slow");
    assert_eq!(out.bad("down"), "down");
    assert_eq!(out.dim("path"), "path");
    assert_eq!(out.cmd("varde up"), "varde up");
    assert_eq!(out.key("API"), "API");
    assert_eq!(out.value("8000"), "8000");
    assert_eq!(out.backticks("run `varde up`"), "run `varde up`");
}

#[test]
fn styling_helpers_emit_escapes_with_colour() {
    let out = colored();
    let painted = out.ok("up");
    assert!(painted.contains('\u{1b}'), "{painted:?}");
    assert_eq!(console::strip_ansi_codes(&painted), "up");
    let hint = out.backticks("start it with `varde up`");
    assert!(hint.contains('\u{1b}'));
    assert_eq!(
        console::strip_ansi_codes(&hint),
        "start it with `varde up`",
        "the backticks stay, so the plain reading never changes"
    );
}

#[test]
fn tracing_is_off_until_a_flag_turns_it_on() {
    let quiet = Out::default();
    assert!(!quiet.is_verbose());
    assert!(!quiet.is_debug());

    let hints = Out {
        verbosity: HINTS,
        ..Out::default()
    };
    assert!(hints.shows_hints());
    assert!(!hints.is_verbose(), "-v shows hints, not the trace");

    let verbose = Out {
        verbosity: VERBOSE,
        ..Out::default()
    };
    assert!(verbose.is_verbose() && verbose.shows_hints());
    assert!(!verbose.is_debug(), "-vv does not print bodies");

    let debug = Out {
        verbosity: DEBUG,
        ..Out::default()
    };
    assert!(debug.is_debug());
    assert!(debug.is_verbose(), "-d implies -vv");
}

#[test]
fn set_verbosity_maps_the_flags_onto_the_levels() {
    assert_eq!(set_verbosity(0, false), QUIET);
    assert_eq!(set_verbosity(1, false), HINTS);
    assert_eq!(set_verbosity(2, false), VERBOSE);
    assert_eq!(set_verbosity(5, false), VERBOSE);
    assert_eq!(set_verbosity(0, true), DEBUG, "-d implies -vv");
    assert_eq!(set_verbosity(1, true), DEBUG);
    // Leave the process as quiet as the other tests expect it.
    set_verbosity(0, false);
}

#[test]
fn a_traced_body_is_cut_at_a_character_boundary() {
    let short = "{\"status\":\"ok\"}";
    assert_eq!(trace_body(short), short);

    let long = "æ".repeat(MAX_TRACE_BODY);
    let cut = trace_body(&long);
    assert!(cut.ends_with(&format!("({} bytes total)", long.len())));
    assert!(cut.starts_with('æ'));
    assert!(
        cut.len() < long.len(),
        "a body that does not fit is cut, not printed whole"
    );
}

#[test]
fn detect_turns_colour_off_for_json_and_no_color() {
    // stdout is not a terminal under `cargo test`, which is the case that
    // matters most: piped output is never painted.
    assert!(!Out::detect(false, false).color);
    assert!(!Out::detect(true, false).color);
    assert!(!Out::detect(false, true).color);
    assert!(!Out::detect(false, false).tty);
    assert!(!Out::detect(true, false).tty || Out::detect(true, false).json);
}

#[test]
fn human_error_lists_causes() {
    let out = Out::default();
    let err = anyhow::anyhow!("root cause")
        .context("middle")
        .context("top");
    assert_eq!(
        out.error(&err),
        "error: top\n  caused by: middle\n  caused by: root cause"
    );
}

#[test]
fn an_error_on_a_terminal_is_the_same_line_with_a_coloured_label() {
    let err = anyhow::anyhow!("compose files are out of date with .varde/; run `varde sync`");
    let plain = Out {
        tty: true,
        ..Out::default()
    }
    .error(&err);
    assert_eq!(
        plain,
        "error: compose files are out of date with .varde/; run `varde sync`"
    );
    let painted = colored().error(&err);
    assert_eq!(console::strip_ansi_codes(&painted), plain);
    assert!(!painted.contains('\u{256d}'), "no box: {painted}");
}

#[test]
fn json_error_is_an_object_with_causes() {
    let out = Out {
        json: true,
        ..Out::default()
    };
    let err = anyhow::anyhow!("root cause").context("top");
    let value: serde_json::Value = serde_json::from_str(&out.error(&err)).unwrap();
    assert_eq!(value["error"], "top");
    assert_eq!(value["causes"], serde_json::json!(["root cause"]));
}

#[test]
fn fields_align_labels_and_skip_empty_values() {
    let text = fields(
        2,
        &[
            ("id", "chapkit_ewars_model".into()),
            ("service", "chapkit-ewars-model".into()),
            ("citation", String::new()),
            ("covariates", "required: rainfall\ndefaults: none".into()),
        ],
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "  id          chapkit_ewars_model");
    assert_eq!(lines[1], "  service     chapkit-ewars-model");
    assert_eq!(lines[2], "  covariates  required: rainfall");
    assert_eq!(lines[3], "              defaults: none");
    assert!(lines.iter().all(|l| !l.ends_with(' ')));
    assert!(!text.contains("citation"));
}

#[test]
fn styled_labels_keep_the_same_column() {
    let out = colored();
    let entries = [
        ("id", "chapkit_ewars_model".to_string()),
        ("service", "chapkit-ewars-model".to_string()),
    ];
    let styled = fields_with(2, &entries, &|label| out.dim(label));
    assert_eq!(
        console::strip_ansi_codes(&styled).to_string(),
        fields(2, &entries)
    );
}

#[test]
fn wrap_breaks_on_words_and_keeps_paragraphs() {
    assert_eq!(
        wrap("one two three four", 9),
        vec!["one two", "three", "four"]
    );
    assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    assert_eq!(
        wrap("ghcr.io/dhis2-chap/chapkit-r-inla", 10),
        vec!["ghcr.io/dhis2-chap/chapkit-r-inla"],
        "a long word is never split"
    );
}

#[test]
fn human_age_uses_the_largest_unit() {
    assert_eq!(human_age(Duration::from_secs(0)), "0 seconds");
    assert_eq!(human_age(Duration::from_secs(1)), "1 second");
    assert_eq!(human_age(Duration::from_secs(90)), "1 minute");
    assert_eq!(human_age(Duration::from_hours(3)), "3 hours");
    assert_eq!(human_age(Duration::from_hours(50)), "2 days");
}

#[test]
fn ago_abbreviates_the_same_units() {
    assert_eq!(ago(Duration::from_secs(0)), "0s ago");
    assert_eq!(ago(Duration::from_secs(12)), "12s ago");
    assert_eq!(ago(Duration::from_secs(59)), "59s ago");
    assert_eq!(ago(Duration::from_secs(180)), "3m ago");
    assert_eq!(ago(Duration::from_hours(3)), "3h ago");
    assert_eq!(ago(Duration::from_hours(50)), "2d ago");
    // The same boundary human_age uses, so the two never disagree.
    assert_eq!(ago(Duration::from_secs(60)), "1m ago");
    assert_eq!(human_age(Duration::from_secs(60)), "1 minute");
}

/// 2026-09-25T13:04:00Z, which is the hour a rate-limit reset lands on.
const RESET: u64 = 1_758_805_440;

#[test]
fn a_clock_time_is_the_local_hour_and_minute() {
    assert_eq!(clock_of(RESET, 0), "13:04");
    // Two hours east and eight hours west of it.
    assert_eq!(clock_of(RESET, 2 * 3600), "15:04");
    assert_eq!(clock_of(RESET, -8 * 3600), "05:04");
    // An offset that is not a whole hour, and one that crosses midnight.
    assert_eq!(clock_of(RESET, 5 * 3600 + 1800), "18:34");
    assert_eq!(clock_of(RESET, 11 * 3600), "00:04");
    // A clock before 1970 is not a thing this prints.
    assert_eq!(clock_of(0, -3600), "00:00");
    // Whatever this machine's zone is, the answer is a clock.
    assert!(
        regex::Regex::new(r"^\d{2}:\d{2}$")
            .unwrap()
            .is_match(&local_clock(RESET)),
        "{}",
        local_clock(RESET)
    );
}

/// The zone file reader, against a version 1 file built here: two
/// transitions, and a type table with the offsets they select.
#[test]
fn the_zone_file_gives_the_offset_in_force() {
    fn tzif(version: u8, transitions: &[(i32, u8)], offsets: &[i32]) -> Vec<u8> {
        let mut out = b"TZif".to_vec();
        out.push(version);
        out.extend(std::iter::repeat_n(0u8, 15));
        for count in [
            0u32,
            0,
            0,
            transitions.len() as u32,
            offsets.len() as u32,
            0,
        ] {
            out.extend(count.to_be_bytes());
        }
        for (when, _) in transitions {
            out.extend(when.to_be_bytes());
        }
        for (_, which) in transitions {
            out.push(*which);
        }
        for offset in offsets {
            out.extend(offset.to_be_bytes());
            out.push(0);
            out.push(0);
        }
        out
    }

    // Standard time until the first transition, summer time after it,
    // standard time again after the second.
    let file = tzif(
        b'\0',
        &[(1_743_296_400, 1), (1_761_440_400, 0)],
        &[3600, 7200],
    );
    assert_eq!(tzif_offset(&file, 1_700_000_000), Some(3600), "before both");
    assert_eq!(tzif_offset(&file, RESET as i64), Some(7200), "in between");
    assert_eq!(tzif_offset(&file, 1_800_000_000), Some(3600), "after both");

    // A zone that never changes carries no transitions at all.
    let fixed = tzif(b'\0', &[], &[-18_000]);
    assert_eq!(tzif_offset(&fixed, RESET as i64), Some(-18_000));

    // Anything that is not a zone file, and a truncated one, are nothing
    // rather than a wrong hour.
    assert_eq!(tzif_offset(b"not a zone file at all", RESET as i64), None);
    assert_eq!(tzif_offset(&file[..30], RESET as i64), None);
    assert_eq!(tzif_offset(&[], 0), None);
}

/// A version 2 file carries the 32-bit table first and the one that is
/// actually read second; `zic` leaves the first empty, and reading it
/// would put every machine on UTC.
#[test]
fn a_version_2_zone_file_is_read_from_its_second_block() {
    let mut out = b"TZif2".to_vec();
    out.extend(std::iter::repeat_n(0u8, 15));
    // The empty 32-bit block: no transitions, one type of no interest.
    for count in [0u32, 0, 0, 0, 1, 0] {
        out.extend(count.to_be_bytes());
    }
    out.extend(0i32.to_be_bytes());
    out.extend([0u8, 0]);
    // The 64-bit block, with the offset that has to win.
    out.extend(b"TZif2");
    out.extend(std::iter::repeat_n(0u8, 15));
    for count in [0u32, 0, 0, 1, 2, 0] {
        out.extend(count.to_be_bytes());
    }
    out.extend(1_743_296_400i64.to_be_bytes());
    out.push(1);
    for offset in [3600i32, 7200] {
        out.extend(offset.to_be_bytes());
        out.extend([0u8, 0]);
    }

    assert_eq!(tzif_offset(&out, RESET as i64), Some(7200));
    assert_eq!(tzif_offset(&out, 1_700_000_000), Some(3600));
}

#[test]
fn hint_of_takes_the_last_clause_that_names_a_command() {
    assert_eq!(
        hint_of("unknown model `x`; run `varde models search x`").as_deref(),
        Some("run `varde models search x`")
    );
    assert_eq!(
        hint_of("a; b; pass `--id <other>` to add it beside it").as_deref(),
        Some("pass `--id <other>` to add it beside it")
    );
    assert_eq!(hint_of("unknown model `x`"), None);
    assert_eq!(hint_of("no command; here"), None);
}

#[test]
fn a_message_of_several_lines_stays_whole_in_the_error() {
    let message = "2 host ports this deployment needs are already in use:\n  \
                   port 8700 ...; stop it with `varde -C /a down`\n  \
                   port 8780 ...; run `varde components enable dhis2 --port 8781`";
    assert_eq!(hint_of(message), None);
    assert_eq!(split_hint(message), (message.to_string(), None));
}

#[test]
fn a_json_error_says_ok_false_and_carries_the_hint_once() {
    let out = Out {
        json: true,
        ..Out::default()
    };
    let err = anyhow::anyhow!("port 8700 is in use; free it, or set `CHAP_API_PORT` in `.env`");
    let value: serde_json::Value = serde_json::from_str(&out.error(&err)).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["hint"], "free it, or set `CHAP_API_PORT` in `.env`");
    assert_eq!(value["error"], "port 8700 is in use");
}

#[test]
fn a_json_error_without_a_way_out_keeps_the_whole_message() {
    let out = Out {
        json: true,
        ..Out::default()
    };
    let err = anyhow::anyhow!("unknown model `x`; nothing else");
    let value: serde_json::Value = serde_json::from_str(&out.error(&err)).unwrap();
    assert_eq!(value["hint"], serde_json::Value::Null);
    assert_eq!(value["error"], "unknown model `x`; nothing else");
}

fn sample_report() -> Report {
    let mut report = Report::default();
    report
        .info("updated the marketplace registry: 7 models")
        .hint("`varde self update` updates varde itself")
        .warning("the cache is old");
    report
}

#[test]
fn a_report_shows_info_and_warnings_by_default() {
    let (stdout, stderr) = sample_report().render(false, false);
    assert_eq!(stdout, "updated the marketplace registry: 7 models\n");
    assert_eq!(stderr, "warning: the cache is old\n");
}

#[test]
fn a_report_shows_the_hints_under_verbose() {
    let (stdout, _) = sample_report().render(true, false);
    assert_eq!(
        stdout,
        "updated the marketplace registry: 7 models\n\
         hint: `varde self update` updates varde itself\n"
    );
}

#[test]
fn every_message_keeps_its_level_for_json() {
    let report = sample_report();
    let json = serde_json::to_value(report.messages()).unwrap();
    assert_eq!(json[0]["level"], "info");
    assert_eq!(json[1]["level"], "hint");
    assert_eq!(json[2]["level"], "warning");
}

/// A warning keeps its place in the report: it comes after the info line
/// above it, on its own stream.
#[test]
fn a_report_keeps_the_order_of_its_lines_across_the_two_streams() {
    use super::report::Stream;
    let mut report = Report::default();
    report
        .info("external DHIS2 2.42.6 at http://localhost:8790")
        .warning("nothing answered through the route")
        .info("run `varde dhis2 show`");
    assert_eq!(
        report.lines(false, false),
        vec![
            (
                Stream::Stdout,
                "external DHIS2 2.42.6 at http://localhost:8790\n".to_string()
            ),
            (
                Stream::Stderr,
                "warning: nothing answered through the route\n".to_string()
            ),
            (Stream::Stdout, "run `varde dhis2 show`\n".to_string()),
        ]
    );
}
