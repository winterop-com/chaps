//! The summary strip under the table: the row under the cursor, or what
//! saving would do.

use super::table::MODEL_W;
use super::text::{fit, list_or_dash, plural, truncate, version_and_tag, with_kept_tail};
use crate::components::Component;
use crate::tui::app::{App, Change, ChangeKind, Page};
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// How many pending changes the summary strip lists before it counts the rest.
const STRIP_CHANGES: usize = 5;

/// How tall the summary strip wants to be.
///
/// One row for the model under the cursor, so the list keeps the rest; only
/// something unsaved makes the strip grow, a line per change.
pub(super) fn strip_height(changes: &[Change]) -> u16 {
    if changes.is_empty() {
        return 1;
    }
    let listed = changes.len().min(STRIP_CHANGES);
    let more = usize::from(changes.len() > STRIP_CHANGES);
    (1 + listed + more) as u16
}

/// The strip under the table: one line for the row under the cursor, or, when
/// there is something unsaved, a line for each thing saving would do.
pub(super) fn draw_strip(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    changes: &[Change],
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // The strip is indented the way the rows are, so the two read as one box.
    let width = area.width.saturating_sub(1) as usize;
    let lines = match (changes.is_empty(), app.page) {
        (false, _) => change_lines(changes, width, theme),
        (true, Page::Models) => summary_lines(app, width, theme),
        (true, Page::Components) => component_summary_lines(app, width, theme),
    };
    let lines = lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect::<Vec<Line>>();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The row under the cursor, in one line: what it is, how far it can be
/// trusted, what it is pinned to, whether this deployment runs it, and what
/// data it needs. Everything else about it is behind `i`.
pub(super) fn summary_lines<'a>(app: &App, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let Some(row) = app.selected() else {
        return vec![Line::from(Span::styled(
            "nothing matches this filter",
            theme.dim_style(),
        ))];
    };
    let model = app.model(row);

    let mut head = vec![
        Span::styled(model.display_name.clone(), theme.accent_style()),
        Span::raw("  "),
        Span::styled("\u{25cf} ", theme.status_style(model.assessed_status)),
        Span::raw(model.assessed_status.label()),
    ];
    let pinned = match app.recorded(row) {
        Some(recorded) => Some(version_and_tag(&recorded.version, &recorded.image_tag)),
        None => app
            .resolved(row)
            .map(|v| version_and_tag(&v.version, &v.image_tag)),
    };
    if let Some(pinned) = pinned {
        head.push(Span::raw("  "));
        head.push(Span::styled(pinned, theme.dim_style()));
    }
    head.push(Span::raw("  "));
    match app.recorded(row) {
        Some(recorded) => head.push(Span::styled(
            match recorded.host_port {
                Some(port) => format!("enabled on port {port}"),
                None => "enabled, via chap-core".to_string(),
            },
            theme.ok_style(),
        )),
        None => head.push(Span::styled("not enabled", theme.dim_style())),
    }
    // Last, because it is the one part that can be any length: a narrow
    // terminal cuts the covariates and keeps everything in front of them.
    head.push(Span::raw("  "));
    head.push(Span::styled("requires ", theme.dim_style()));
    head.push(Span::raw(list_or_dash(&model.covariates.required)));

    vec![Line::from(with_kept_tail(
        head,
        Span::styled("i for details", theme.dim_style()),
        width,
    ))]
}

/// The component under the cursor, in one line: what it is called, whether
/// this deployment has it, where it is reached, and the file `sync` renders
/// for it. The rest is behind `i`.
fn component_summary_lines<'a>(app: &App, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let component = app.selected_component();
    let enabled = app.components.is_enabled(component);
    let mut head = vec![
        Span::styled(component.name().to_string(), theme.accent_style()),
        Span::raw("  "),
        Span::styled(
            match enabled {
                true => "enabled here",
                false => "not enabled here",
            },
            match enabled {
                true => theme.ok_style(),
                false => theme.dim_style(),
            },
        ),
        Span::raw("  "),
        Span::styled(app.component_reach(component), theme.dim_style()),
        Span::raw("  "),
        Span::styled(compose_of(component), theme.dim_style()),
    ];
    // Last, because it is the one part that can be any length.
    head.push(Span::raw("  "));
    head.push(Span::raw(component.summary().to_string()));
    vec![Line::from(with_kept_tail(
        head,
        Span::styled("i for details", theme.dim_style()),
        width,
    ))]
}

/// The compose file `varde sync` renders for a component.
///
/// chap-core's are upstream's own, which this CLI renders but does not author,
/// so they are named as the pair they are.
pub(super) fn compose_of(component: Component) -> String {
    match component {
        Component::ChapCore => format!(
            "{} + {}",
            crate::project::BASE_COMPOSE,
            crate::project::VARDE_COMPOSE
        ),
        Component::Ocs => crate::components::OCS_COMPOSE.to_string(),
        Component::S3 => crate::components::S3_COMPOSE.to_string(),
        Component::Dhis2 => crate::components::DHIS2_COMPOSE.to_string(),
    }
}

/// What saving would write, one line per model.
fn change_lines<'a>(changes: &[Change], width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} pending change{}", changes.len(), plural(changes.len())),
            theme.warn_style(),
        ),
        Span::raw("  "),
        Span::styled("nothing is written until you save", theme.dim_style()),
    ])];
    for change in changes.iter().take(STRIP_CHANGES) {
        let (mark, style) = match change.kind {
            ChangeKind::Add => ("+", theme.ok_style()),
            ChangeKind::Update => ("+", theme.warn_style()),
            ChangeKind::Remove => ("-", theme.bad_style()),
        };
        let mut spans = vec![
            Span::styled(format!("{mark} "), style),
            Span::raw(fit(&change.name, MODEL_W)),
            Span::raw("  "),
            Span::styled(change.detail.clone(), theme.dim_style()),
        ];
        truncate(&mut spans, width);
        lines.push(Line::from(spans));
    }
    if changes.len() > STRIP_CHANGES {
        lines.push(Line::from(Span::styled(
            format!("  and {} more", changes.len() - STRIP_CHANGES),
            theme.dim_style(),
        )));
    }
    lines
}
