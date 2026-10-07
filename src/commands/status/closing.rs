//! The lines under the rows of `varde status`: the one line they add up to,
//! a line per row that needs a fix, and the hints.

use super::{INTERNAL, NOT_RUNNING};
use crate::output::Report;
use crate::status::{ApiHealth, StatusReport, closing_line, hints};

/// Which "nothing is running" line this deployment gets.
///
/// [`ApiHealth::Off`] is exactly "chap-core is not a component of this
/// deployment", so the report already carries the answer and the choice needs
/// no project: a deployment that has no Chap in it is never told that Chap is
/// not running.
pub(super) fn nothing_running_line(report: &StatusReport) -> &'static str {
    match report.api {
        ApiHealth::Off if report.components.is_empty() && report.models.is_empty() => {
            crate::status::EMPTY
        }
        ApiHealth::Off => crate::status::NOTHING_RUNNING,
        _ => NOT_RUNNING,
    }
}

/// What a deployment with no containers at all is told.
pub(super) fn not_running_lines(report: &StatusReport, lines: &mut Report) {
    lines.info(nothing_running_line(report));
    // The port answering anyway is the one thing worth adding: it is why a
    // browser on it shows a Chap while this says none is running.
    if let Some(why) = report.api_elsewhere.as_deref() {
        lines.info(why);
    }
}

/// The lines under the rows: the one line they add up to, a line per row that
/// needs a fix, and the optional next commands as hints.
///
/// `external` is a chap-core that this deployment does not run, where a model
/// that does not register has a cause of its own.
pub(super) fn closing(report: &StatusReport, external: bool, lines: &mut Report) {
    // A deployment chap-core is not a component of has no API to be down and
    // no models to register, so its components are its verdict. Without this
    // it would be the one deployment shape `varde status` said nothing about
    // at the end, and `closing_line` would name `varde models enable`, which
    // is refused there.
    if matches!(report.api, ApiHealth::Off) {
        for line in crate::status::standalone_closing_lines(&report.models, &report.components) {
            lines.info(line);
        }
        for hint in crate::status::standalone_hints(&report.models, &report.components) {
            lines.hint(hint);
        }
        return;
    }

    // Everything the API cannot be asked about is left out while it is down:
    // one error line beats a table's worth of consequences.
    if !matches!(report.api, ApiHealth::Up { .. }) {
        return;
    }

    lines.info(closing_line(&report.models));
    let elsewhere = report
        .chap_core_elsewhere
        .then_some(report.api_url.as_str());
    for fix in hints(&report.models, report.auth, elsewhere) {
        lines.info(fix);
    }
    if let Some(connect) = connect_hint(report) {
        lines.info(connect);
    }
    if external {
        let fixes = crate::status::external_registration_hints(&report.models);
        if !fixes.is_empty() {
            for fix in fixes {
                lines.info(fix);
            }
            lines.hint(crate::status::EXTERNAL_REGISTRATION_LOG);
        }
    }
    // The REACH column says `via chap-core` for a model with no host port of
    // its own; the way in is said once, here, rather than in every row.
    if report.models.iter().any(|m| m.reach == INTERNAL) {
        lines.hint(format!(
            "models without a host port are reachable through chap-core at \
             {}/v2/services/<id>/run/",
            report.api_url
        ));
    }
    if let Some(test) = crate::status::test_hint(&report.models) {
        lines.hint(test);
    }
}

/// The hint that this deployment's DHIS2 has still to be connected to Chap,
/// for a run where it is worth acting on.
///
/// Two conditions, and the second is the one worth arguing about. The first is
/// [`StatusReport::dhis2_needs_connecting`], which is `.varde/components.yaml`
/// and no request at all. The second is that the `dhis2` row says `up`: telling
/// someone to connect to a DHIS2 that is not running is advice they cannot
/// take, and `varde status` has just asked `/api/ping` and knows the answer, so
/// the line waits for the run where the command it names would work. A DHIS2
/// that is `starting` is one whose API is not answering yet, which is exactly
/// the wait `varde dhis2 connect` would sit in.
///
/// It sits with the model fixes, under the verdict, because it is the same kind
/// of thing: one line per row that still needs something done about it.
fn connect_hint(report: &crate::status::StatusReport) -> Option<String> {
    if !report.dhis2_needs_connecting {
        return None;
    }
    let up = report.components.iter().any(|component| {
        component.name == crate::compose::DHIS2_SERVICE
            && component.state == crate::status::ComponentState::Up
    });
    up.then(|| crate::components::dhis2_connect_hint(true))
}
