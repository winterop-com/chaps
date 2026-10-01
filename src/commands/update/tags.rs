//! `chaps update --list-tags`: where a deployment can move chap-core to.

use crate::chapcore;
use crate::commands::Ctx;
use crate::compose::API_SERVICE;
use crate::docker;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::Project;
use serde::Serialize;
use std::collections::BTreeMap;

/// What `chaps update --list-tags` prints, and the whole of its `--json`.
#[derive(Debug, Serialize)]
pub struct TagList {
    /// The tag `.chaps/project.yaml` records today.
    pub pin: String,
    /// Whether the releases could be listed at all. False is `--offline` or a
    /// lookup that did not arrive, and the table is then the moving tags plus
    /// this deployment's own pin.
    pub releases_listed: bool,
    pub tags: Vec<TagRow>,
}

/// One tag a deployment can move chap-core to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagRow {
    pub tag: String,
    /// `release`, `moving`, or `exact` for a pin that is neither.
    pub kind: &'static str,
    /// The day the release was published, or the branch behind a moving tag
    /// was last committed to. `null` when it could not be had cheaply.
    pub published: Option<String>,
    /// Whether this is the tag the deployment records today.
    pub pinned: bool,
    /// Whether this is the newest release.
    pub newest: bool,
    /// Which build a pinned moving tag is actually running, when its container
    /// is up and docker could be asked.
    pub running: Option<String>,
}

/// The rows of the listing: the moving tags, then the releases newest first,
/// and this deployment's pin when it is neither.
pub fn tag_rows(
    pin: &str,
    releases: &[chapcore::Release],
    published: &BTreeMap<String, String>,
    running: Option<&str>,
) -> Vec<TagRow> {
    let row = |tag: &str, kind: &'static str, day: Option<String>, newest: bool| {
        let pinned = tag == pin;
        TagRow {
            tag: tag.to_string(),
            kind,
            published: day.filter(|d| !d.is_empty()),
            pinned,
            newest,
            // Which image a release tag names is not in question, so the
            // digest is only ever printed where the tag does not say.
            running: (pinned && kind == chapcore::TagKind::Moving.label())
                .then(|| running.map(str::to_string))
                .flatten(),
        }
    };
    let mut rows: Vec<TagRow> = chapcore::MOVING_TAGS
        .iter()
        .map(|tag| {
            row(
                tag,
                chapcore::TagKind::Moving.label(),
                published.get(*tag).cloned(),
                false,
            )
        })
        .collect();

    let mut sorted: Vec<&chapcore::Release> = releases.iter().collect();
    sorted.sort_by_key(|release| std::cmp::Reverse(chapcore::release_version(&release.tag)));
    for (index, release) in sorted.iter().enumerate() {
        rows.push(row(
            &release.tag,
            chapcore::TagKind::Release.label(),
            Some(release.published.clone()),
            index == 0,
        ));
    }
    // A pin the listing does not otherwise hold - an older release, or a
    // `sha-` build - is still where this deployment is, and the point of the
    // table is to say where it can go from there.
    if !rows.iter().any(|row| row.tag == pin) {
        rows.push(row(pin, chapcore::tag_kind(pin).label(), None, false));
    }
    rows
}

/// The NOTE cell: what this row is to this deployment, and to the release
/// line. `-` for a tag that is neither pinned nor the newest.
pub fn tag_note(row: &TagRow) -> String {
    let mut parts = Vec::new();
    if row.pinned {
        parts.push(match &row.running {
            Some(digest) => format!("pinned (moving, running {digest})"),
            None => "pinned".to_string(),
        });
    }
    if row.newest {
        parts.push("newest".to_string());
    }
    if parts.is_empty() {
        return "-".to_string();
    }
    parts.join(", ")
}

/// The one line the listing ends on: what to do with a tag from the table.
pub fn tag_closing_line(list: &TagList) -> String {
    format!(
        "chap-core is pinned to {}; move it with `chaps update --chap-tag <TAG>`",
        list.pin
    )
}

/// The table, then that line.
fn tag_table(out: &Out, list: &TagList) -> String {
    let rows: Vec<Vec<String>> = list
        .tags
        .iter()
        .map(|row| {
            let note = tag_note(row);
            vec![
                row.tag.clone(),
                out.dim(row.kind),
                out.dim(row.published.as_deref().unwrap_or("-")),
                if row.pinned {
                    out.ok(&note)
                } else {
                    out.dim(&note)
                },
            ]
        })
        .collect();
    let mut text = out.table(&["TAG", "KIND", "PUBLISHED", "NOTE"], &rows);
    text.push('\n');
    text.push_str(&out.cmd(&out.backticks(&tag_closing_line(list))));
    text
}

/// `chaps update --list-tags`: where this deployment can move chap-core to.
///
/// It writes nothing and pulls nothing, so it is also the command to run
/// before `--chap-tag`, and it works offline: the three moving tags and this
/// deployment's own pin need nothing from the network.
pub(super) fn list_tags(ctx: &Ctx, project: &Project) -> Result<()> {
    let pin = project.state.chap_image_tag.clone();
    let timeout = ctx.registry.timeout;
    let offline = ctx.registry.offline;
    let releases = if offline {
        output::warn(
            "--offline: the chap-core releases were not listed, so this is the moving tags and \
             this deployment's own pin",
        );
        Vec::new()
    } else {
        match chapcore::releases(chapcore::LIST_LIMIT, timeout) {
            Ok(list) => list,
            Err(err) => {
                output::warn(&format!(
                    "could not list the chap-core releases ({err:#}), so this is the moving tags \
                     and this deployment's own pin"
                ));
                Vec::new()
            }
        }
    };

    // What each moving tag was last built from, where it is one request away:
    // `dev` and `master` from their branches, and `latest` from the release
    // that publishes it. A branch that will not answer costs its own cell.
    let mut published = BTreeMap::new();
    if !offline {
        for tag in chapcore::MOVING_TAGS {
            if *tag == chapcore::LATEST_TAG {
                continue;
            }
            if let Some(day) = chapcore::branch_updated(tag, timeout) {
                published.insert((*tag).to_string(), day);
            }
        }
    }
    if let Some(newest) = releases.first() {
        published.insert(chapcore::LATEST_TAG.to_string(), newest.published.clone());
    }

    // Which build the pin is on, for the one kind of tag that does not say.
    let running = chapcore::is_moving_tag(&pin)
        .then(|| {
            docker::running_containers(project)
                .as_deref()
                .and_then(|containers| docker::running_build(containers, API_SERVICE))
        })
        .flatten();

    let list = TagList {
        releases_listed: !releases.is_empty(),
        tags: tag_rows(&pin, &releases, &published, running.as_deref()),
        pin,
    };
    ctx.out.emit(&list, || tag_table(&ctx.out, &list))
}
