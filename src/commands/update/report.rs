//! What the run prints: the plan, and the one line it ends on.

use super::UpdateReport;
use super::chap_core::chap_core_cell;
use super::components::component_line;
use super::models::ModelUpdate;
use crate::output::Out;

/// What this run actually changed, as the closing line names it, or `None`
/// when the answer is "nothing".
///
/// Three clauses at most, in the order they matter: the pins that moved, and
/// then the images that arrived. A moving tag that pulled the same image
/// again is not a change and says nothing here, which is the whole point of
/// asking the image ids rather than trusting that a pull did something.
pub fn updated_phrase(
    models: usize,
    chap_core: Option<(&str, &str)>,
    pulled_new: &[String],
) -> Option<String> {
    let mut parts = Vec::new();
    if models > 0 {
        parts.push(format!(
            "{models} model pin{}",
            if models == 1 { "" } else { "s" }
        ));
    }
    if let Some((old, new)) = chap_core {
        parts.push(format!("chap-core {old} -> {new}"));
    }
    let pins = (!parts.is_empty()).then(|| parts.join(", "));
    // The images are a clause of their own, with its own verb: a pull is not
    // an update of anything, and "updated new image for ocs" is not English.
    let images = (!pulled_new.is_empty()).then(|| match pulled_new {
        [one] => format!("pulled a new image for {one}"),
        many => format!("pulled new images for {}", many.join(", ")),
    });
    match (pins, images) {
        (Some(pins), Some(images)) => Some(format!("{pins} and {images}")),
        (pins, images) => pins.or(images),
    }
}

/// [`updated_phrase`] for a finished report.
pub(super) fn updated(report: &UpdateReport) -> Option<String> {
    let chap_core = report
        .chap_core
        .as_ref()
        .filter(|c| c.changed)
        .map(|c| (c.old_tag.as_str(), c.new_tag.as_str()));
    updated_phrase(report.changed().count(), chap_core, &report.pulled_new)
}

/// The one line the run ends on.
///
/// It answers the only question left: is there anything to do now? A stale
/// running service is that answer whether or not this run is what made it
/// stale, so the restart clause comes first and stands on its own.
pub fn closing_line(
    what: Option<&str>,
    restart_needed: &[String],
    running: Option<bool>,
) -> String {
    let head = match what {
        // A run that only pulled says so in its own verb.
        Some(what) if what.starts_with("pulled ") => what.to_string(),
        Some(what) => format!("updated {what}"),
        None => "already up to date".to_string(),
    };
    if !restart_needed.is_empty() {
        return format!(
            "{head}; restart needed: {} (run `chaps restart`)",
            restart_needed.join(", ")
        );
    }
    if what.is_none() {
        return head;
    }
    match running {
        Some(false) => {
            format!("{head}; Chap is not running, the new versions start with `chaps up`")
        }
        Some(true) => format!("{head}; nothing needs a restart"),
        // Docker would not say what is running, so neither will we.
        None => format!("{head}; run `chaps restart` to apply it to whatever is running"),
    }
}

/// The one line a `--dry-run` ends on.
///
/// It says what the pins would do and nothing about restarting: the images
/// were not pulled and the files were not written, so what a running
/// container would then be out of step with does not exist yet.
pub fn dry_run_line(what: Option<&str>) -> String {
    match what {
        Some(what) => format!("would update {what}; nothing written (run `chaps update` to do it)"),
        None => "already up to date; nothing would change".to_string(),
    }
}

/// The plan: where the catalogue came from, and a row per thing that can move.
pub(super) fn plan_text(report: &UpdateReport, out: &Out) -> String {
    let mut text = format!(
        "{} {} {}\n",
        out.key("registry:"),
        report.registry.url,
        out.dim(&format!("({})", report.registry.provenance.describe()))
    );
    if report.models.is_empty() {
        text.push_str(&out.dim("no models enabled"));
        text.push('\n');
    }
    for m in &report.models {
        // The new version is the only thing on the line that changed, so it is
        // the only thing that is coloured.
        let old = version_cell(m, &m.old_version, &m.old_tag);
        let line = if m.changed {
            format!(
                "  {}  {} -> {}",
                m.id,
                out.dim(&old),
                out.ok(&version_cell(m, &m.new_version, &m.new_tag))
            )
        } else {
            format!("  {}  {}  {}", m.id, out.dim(&old), out.dim(state_of(m)))
        };
        text.push_str(&line);
        text.push('\n');
        if let Some(user) = user_line(m) {
            text.push_str(&format!("    {}\n", out.dim(&user)));
        }
    }
    let tense = |line: String| would(line, report.dry_run);
    if let Some(core) = &report.chap_core {
        text.push_str(&format!("  {}\n", tense(chap_core_cell(out, core))));
    }
    for component in &report.components {
        text.push_str(&format!(
            "  {}\n",
            out.dim(&tense(component_line(component)))
        ));
    }
    text
}

/// A plan line in the tense of the run: a dry run pulls nothing, so a moving
/// tag there *would be* re-pulled rather than being said to have been.
pub(super) fn would(line: String, dry_run: bool) -> String {
    if dry_run {
        line.replace("re-pulled", "would be re-pulled")
    } else {
        line
    }
}

/// What a row says about the account the model runs as, when the new tag
/// changed it: `user 1000:1000 -> root (image config)`.
///
/// Nothing at all when it did not, which is almost every row: an image that
/// kept its `USER` line has nothing to report here.
pub(super) fn user_line(m: &ModelUpdate) -> Option<String> {
    let (old, new) = m.moved_user()?;
    Some(format!("user {old} -> {new} ({})", m.user_from.label()))
}

/// How one row's pin reads: `v1.0.0 (sha-fa880a1)` for a marketplace model,
/// and the tag alone for a manual one, whose version is its tag.
pub(super) fn version_cell(m: &ModelUpdate, version: &str, tag: &str) -> String {
    if m.manual {
        return tag.to_string();
    }
    format!("v{version} ({tag})")
}

/// What a row that did not move says for itself.
pub(super) fn state_of(m: &ModelUpdate) -> &'static str {
    if m.pinned {
        return "pinned, skipped";
    }
    if !m.checked {
        return "unchanged (could not check)";
    }
    "unchanged"
}
