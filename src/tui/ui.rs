//! TUI rendering: title bar, the model or component table, the summary strip,
//! key bar and the overlays.
//!
//! Drawing never mutates the [`App`] except for one thing it alone can know:
//! how far the details overlay can scroll, which depends on the size of the
//! terminal. Everything else on screen is derived from the state the reducer
//! produced, and every colour comes from the [`Theme`]. The layout degrades on
//! narrow terminals by dropping columns rather than wrapping or panicking.

use crate::components::Component;
use crate::registry::{Channel, Provenance, Version, VersionStatus};
use crate::tui::app::{App, Change, ChangeKind, Command, ComponentLine, Mode, Page, PortWant, Row};
use crate::tui::keys::{self, Hint};
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};

/// The label column in the details overlay.
const LABEL_WIDTH: usize = 12;

/// A space, the cursor column, and the enabled mark, each with a space.
const MARKER_W: usize = 5;
const MODEL_W: usize = 22;
/// `template` is the longest thing the kind column says.
const KIND_W: usize = 8;
/// `● experimental`: the dot, and the longest thing the scale means.
const STATUS_W: usize = 14;
const VERSION_W: usize = 8;
/// `will be disabled`, the longest thing the port column says.
const PORT_W: usize = 16;
/// Enough for `via chap-core`, when the full column does not fit.
const PORT_MIN: usize = 13;
const ID_MIN: usize = 10;
/// The components page's columns: the name, whether this deployment has it,
/// and where it is reached. What it is takes the rest of the line.
const COMPONENT_W: usize = 12;
const STATE_W: usize = 8;
const REACH_W: usize = 24;
/// Enough for `internal`, or the start of a URL, when the full column does
/// not fit: where a component is reached is worth more than the sentence
/// saying what it is.
const REACH_MIN: usize = 10;
/// The id column stops growing here: a table that stretches an id across a
/// wide terminal only puts distance between the columns that matter.
const ID_MAX: usize = 34;

/// How many pending changes the summary strip lists before it counts the rest.
const STRIP_CHANGES: usize = 5;

/// Draw one frame.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let [title_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_title(frame, title_area, app, theme);
    draw_body(frame, body_area, app, theme);
    draw_footer(frame, footer_area, app, theme);

    match app.mode {
        Mode::Help => draw_help(frame, area, theme),
        Mode::ConfirmQuit => draw_confirm(frame, area, theme),
        Mode::Info => draw_info(frame, area, app, theme),
        Mode::Palette => draw_palette(frame, area, app, theme),
        Mode::Port => draw_port_dialog(frame, area, app, theme),
        Mode::Channel => draw_channel_dialog(frame, area, app, theme),
        Mode::Dhis2Version => draw_dhis2_version_dialog(frame, area, app, theme),
        _ => {}
    }
}

/// `chaps · models` on the left, where the catalogue came from and how big it
/// is on the right. Both halves shrink out of the way before they collide.
fn draw_title(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let counts = app.counts();
    let mut left = vec![
        Span::styled("chaps", theme.accent_style()),
        Span::styled(" · ", theme.dim_style()),
        Span::styled(app.page.title(), theme.label_style()),
    ];
    // A filter is part of what is being looked at, so it sits with the title
    // rather than in a corner, with the cursor where the next letter lands.
    if !app.filter.is_empty() || app.mode == Mode::Filter {
        left.push(Span::styled(" · filter ", theme.dim_style()));
        left.push(Span::styled(app.filter.clone(), theme.label_style()));
        if app.mode == Mode::Filter {
            left.push(Span::styled("_", theme.accent_style()));
        }
    }

    let provenance = Span::styled(registry_label(&app.registry.provenance), theme.dim_style());
    // The counts are about the page in front of the reader; the pending count
    // is both pages', because one save writes both.
    let totals = Span::styled(
        match app.page {
            Page::Models => format!("{} models · {} enabled · ", counts.total, counts.enabled),
            Page::Components => format!(
                "{} components · {} enabled · ",
                Component::ALL.len(),
                app.components.enabled().len()
            ),
        },
        theme.dim_style(),
    );
    let pending = Span::styled(
        format!("{} pending", counts.pending),
        if counts.pending > 0 {
            theme.warn_style()
        } else {
            theme.dim_style()
        },
    );

    // Widest right-hand side that still fits, then the next widest, then none.
    let gap = Span::raw("   ");
    let full = vec![
        provenance.clone(),
        gap.clone(),
        totals.clone(),
        pending.clone(),
    ];
    let just_counts = vec![totals, pending];
    let just_registry = vec![provenance];
    let room = area.width as usize - width_of(&left).min(area.width as usize);
    let right = [full, just_counts, just_registry]
        .into_iter()
        .find(|spans| width_of(spans) + 2 <= room)
        .unwrap_or_default();

    let mut spans = left;
    if !right.is_empty() {
        let pad = area.width as usize - width_of(&spans) - width_of(&right);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// `registry: cache · 2 min`: where the catalogue came from, and how old it is
/// when that is a thing it can be.
fn registry_label(provenance: &Provenance) -> String {
    match provenance {
        Provenance::Cache { age_secs } | Provenance::StaleCache { age_secs } => format!(
            "registry: {} · {}",
            provenance.label(),
            short_age(*age_secs)
        ),
        _ => format!("registry: {}", provenance.label()),
    }
}

/// An age in the width a title bar can spare: `40 sec`, `2 min`, `3 hr`, `5 d`.
fn short_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} sec"),
        60..=3599 => format!("{} min", secs / 60),
        3600..=86_399 => format!("{} hr", secs / 3600),
        _ => format!("{} d", secs / 86_400),
    }
}

/// The bordered box: the column headings, the table, and the summary strip
/// under a rule.
fn draw_body(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = pane(theme, &pane_title(app), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let changes = app.changes();
    let want_strip = strip_height(&changes);
    let [head, rule_top, list, rule_bottom, strip] =
        split_body(inner.height, want_strip).map(Constraint::Length);
    let [head, rule_top, list, rule_bottom, strip] =
        Layout::vertical([head, rule_top, list, rule_bottom, strip]).areas(inner);

    match app.page {
        Page::Models => {
            let columns = columns(inner.width as usize, kind_column(app));
            draw_head(frame, head, &columns, theme);
            draw_rule(frame, rule_top, theme);
            draw_list(frame, list, app, &columns, theme);
        }
        Page::Components => {
            let columns = component_columns(inner.width as usize);
            draw_component_head(frame, head, &columns, theme);
            draw_rule(frame, rule_top, theme);
            draw_component_list(frame, list, app, &columns, theme);
        }
    }
    draw_rule(frame, rule_bottom, theme);
    draw_strip(frame, strip, app, &changes, theme);
}

/// `Marketplace` or `Components`, plus what a filter left of it.
fn pane_title(app: &App) -> String {
    if app.page != Page::Models || app.filter.is_empty() {
        return app.page.pane().to_string();
    }
    format!(
        "Marketplace · {} of {} match \"{}\"",
        app.visible.len(),
        app.rows.len(),
        app.filter
    )
}

/// How tall the summary strip wants to be.
///
/// One row for the model under the cursor, so the list keeps the rest; only
/// something unsaved makes the strip grow, a line per change.
fn strip_height(changes: &[Change]) -> u16 {
    if changes.is_empty() {
        return 1;
    }
    let listed = changes.len().min(STRIP_CHANGES);
    let more = usize::from(changes.len() > STRIP_CHANGES);
    (1 + listed + more) as u16
}

/// Split the box between headings, rules, the table and the strip.
///
/// The table keeps at least one row, the strip comes next, then the headings,
/// and the rules are the first thing a short terminal gives up.
fn split_body(inner_h: u16, want_strip: u16) -> [u16; 5] {
    let mut left = inner_h;
    let list = 1.min(left);
    left -= list;
    let strip = want_strip.min(left);
    left -= strip;
    let head = 1.min(left);
    left -= head;
    let rule_top = if head == 1 { 1.min(left) } else { 0 };
    left -= rule_top;
    let rule_bottom = if strip > 0 { 1.min(left) } else { 0 };
    left -= rule_bottom;
    [head, rule_top, list + left, rule_bottom, strip]
}

/// The widths of the table's columns, so the headings and the rows agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    model: usize,
    kind: usize,
    id: usize,
    status: usize,
    version: usize,
    port: usize,
}

/// Hand out the width the table has, widest column last.
///
/// Every column but the name is optional: they switch on in the order they
/// stop costing the id its readable width, so a narrow terminal loses whole
/// columns instead of wrapping a row.
fn columns(inner: usize, kind: bool) -> Columns {
    let avail = inner.saturating_sub(MARKER_W);
    let model = MODEL_W.min(avail);
    let mut left = avail - model;

    let mut take = |want: usize, min: usize| {
        let width = if left > want {
            want
        } else if left > min && min > 0 {
            min
        } else {
            0
        };
        if width > 0 {
            left -= width + 1;
        }
        width
    };
    let version = take(VERSION_W, 0);
    let status = take(STATUS_W, 0);
    // The enabled column reserves its narrow form first, so the id - the one
    // column the other commands are typed from - is not what an eighty-column
    // terminal gives up.
    let mut port = take(PORT_MIN, 0);
    let kind = if kind { take(KIND_W, 0) } else { 0 };
    let id = if left > ID_MIN {
        (left - 1).min(ID_MAX)
    } else {
        0
    };
    if id > 0 {
        left -= id + 1;
    }
    if port > 0 {
        let grow = (PORT_W - PORT_MIN).min(left);
        port += grow;
    }
    Columns {
        model,
        kind,
        id,
        status,
        version,
        port,
    }
}

/// A kind column appears once the list holds something that is not a plain
/// marketplace model: a template the user asked to see, or an entry this
/// deployment added itself.
fn kind_column(app: &App) -> bool {
    app.show_templates || app.rows.iter().any(|row| app.model(row).manual)
}

/// `MODEL  ID  STATUS  VERSION  ENABLED`, over the columns they label.
fn draw_head(frame: &mut Frame, area: Rect, columns: &Columns, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let mut text = " ".repeat(MARKER_W);
    for (width, label) in [
        (columns.model, "MODEL"),
        (columns.kind, "KIND"),
        (columns.id, "ID"),
        (columns.status, "STATUS"),
        (columns.version, "VERSION"),
        (columns.port, "PORT"),
    ] {
        if width == 0 {
            continue;
        }
        text.push_str(&fit(label, width));
        text.push(' ');
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme.column_style()))),
        area,
    );
}

/// The thin line between the headings and the rows, and between the rows and
/// the summary strip.
fn draw_rule(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            theme.dim_style(),
        ))),
        area,
    );
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App, columns: &Columns, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    if app.visible.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  nothing matches this filter",
                theme.dim_style(),
            )),
            area,
        );
        return;
    }

    let selected = app.cursor.min(app.visible.len() - 1);
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .enumerate()
        .map(|(i, row_idx)| {
            ListItem::new(row_line(
                app,
                &app.rows[*row_idx],
                columns,
                i == selected,
                theme,
            ))
        })
        .collect();

    let list = List::new(items).highlight_style(theme.selection_style());
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

/// One table row: `▸ ✓ Display Name   id   ● status   1.0.3   internal`.
fn row_line<'a>(
    app: &App,
    row: &Row,
    columns: &Columns,
    selected: bool,
    theme: &Theme,
) -> Line<'a> {
    let model = app.model(row);
    let recorded = app.recorded(row).is_some();

    // What the row will be, against what the project has on disk: a change
    // nobody has saved yet is the thing to see first.
    let (mark, mark_style) = match (recorded, row.enabled) {
        (false, true) => ("+", theme.ok_style()),
        (true, false) => ("-", theme.bad_style()),
        (_, true) => ("✓", theme.ok_style()),
        (_, false) => (" ", theme.dim_style()),
    };

    let mut spans = vec![
        Span::styled(if selected { " ▸ " } else { "   " }, theme.accent_style()),
        Span::styled(format!("{mark} "), mark_style),
        Span::raw(fit(&model.display_name, columns.model)),
        Span::raw(" "),
    ];
    if columns.kind > 0 {
        let (label, style) = if model.is_template() {
            ("template", theme.template_style())
        } else if model.manual {
            ("manual", theme.dim_style())
        } else {
            ("model", theme.dim_style())
        };
        spans.push(Span::styled(fit(label, columns.kind), style));
        spans.push(Span::raw(" "));
    }
    if columns.id > 0 {
        spans.push(Span::styled(fit(&model.id, columns.id), theme.dim_style()));
        spans.push(Span::raw(" "));
    }
    if columns.status > 0 {
        spans.push(Span::styled(
            "● ",
            theme.status_style(model.assessed_status),
        ));
        spans.push(Span::raw(fit(
            model.assessed_status.label(),
            columns.status - 2,
        )));
        spans.push(Span::raw(" "));
    }
    if columns.version > 0 {
        let version = app
            .resolved(row)
            .map(|v| v.version.clone())
            .unwrap_or_else(|| "-".to_string());
        spans.push(Span::styled(
            fit(&version, columns.version),
            theme.dim_style(),
        ));
        spans.push(Span::raw(" "));
    }
    if columns.port > 0 {
        let (text, style) = port_cell(row, recorded, columns.port, theme);
        spans.push(Span::styled(fit(&text, columns.port), style));
    }
    Line::from(spans)
}

/// What the port column says: a pending change first, then how an enabled
/// model is reached. A model without a host port of its own is reachable
/// through chap-core and nowhere else, and the port of a row `p` has just
/// asked for is only picked when the selection is applied, hence `auto`.
fn port_cell(
    row: &Row,
    recorded: bool,
    width: usize,
    theme: &Theme,
) -> (String, ratatui::style::Style) {
    // A column too narrow for the phrase says it in one word rather than
    // cutting it off; the summary strip spells it out either way.
    let roomy = width >= PORT_W;
    match (recorded, row.enabled) {
        (false, true) => match roomy {
            true => ("will be enabled".to_string(), theme.ok_style()),
            false => ("enabling".to_string(), theme.ok_style()),
        },
        (true, false) => match roomy {
            true => ("will be disabled".to_string(), theme.bad_style()),
            false => ("disabling".to_string(), theme.bad_style()),
        },
        (_, true) => match row.want {
            PortWant::Exact(port) => (port.to_string(), theme.accent_style()),
            PortWant::Auto => ("auto".to_string(), theme.warn_style()),
            PortWant::None => ("via chap-core".to_string(), theme.dim_style()),
        },
        (_, false) => (String::new(), theme.dim_style()),
    }
}

/// The widths of the components table, so its headings and rows agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ComponentColumns {
    component: usize,
    state: usize,
    reach: usize,
    what: usize,
}

/// Hand out the width the components table has.
///
/// The name, the state and where it is reached come first, and the sentence
/// saying what the component is takes whatever is left: a narrow terminal
/// loses the end of that rather than losing which row is which or where it
/// answers.
fn component_columns(inner: usize) -> ComponentColumns {
    let avail = inner.saturating_sub(MARKER_W);
    let component = COMPONENT_W.min(avail);
    let mut left = avail - component;
    let mut take = |want: usize, min: usize| {
        let width = if left > want {
            want
        } else if left > min && min > 0 {
            min
        } else {
            0
        };
        if width > 0 {
            left -= width + 1;
        }
        width
    };
    let state = take(STATE_W, 0);
    let reach = take(REACH_W, REACH_MIN);
    let what = left.saturating_sub(1);
    ComponentColumns {
        component,
        state,
        reach,
        what,
    }
}

/// `COMPONENT  STATE  REACH  WHAT IT IS`, the columns `components list` prints.
fn draw_component_head(frame: &mut Frame, area: Rect, columns: &ComponentColumns, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let mut text = " ".repeat(MARKER_W);
    for (width, label) in [
        (columns.component, "COMPONENT"),
        (columns.state, "STATE"),
        (columns.reach, "REACH"),
        (columns.what, "WHAT IT IS"),
    ] {
        if width == 0 {
            continue;
        }
        text.push_str(&fit(label, width));
        text.push(' ');
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme.column_style()))),
        area,
    );
}

fn draw_component_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    columns: &ComponentColumns,
    theme: &Theme,
) {
    if area.height == 0 {
        return;
    }
    let lines = app.component_lines();
    let selected = app.component_cursor.min(lines.len().saturating_sub(1));
    let items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| ListItem::new(component_row_line(line, columns, i == selected, theme)))
        .collect();

    let list = List::new(items).highlight_style(theme.selection_style());
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

/// One components row: `▸ ✓ ocs  enabled  http://localhost:8790  Open ...`.
///
/// The mark is the model page's: a tick for what this deployment has, `+` for
/// one this session switched on, `-` for one it switched off.
fn component_row_line<'a>(
    line: &ComponentLine,
    columns: &ComponentColumns,
    selected: bool,
    theme: &Theme,
) -> Line<'a> {
    let (mark, mark_style) = match (line.recorded, line.enabled) {
        (false, true) => ("+", theme.ok_style()),
        (true, false) => ("-", theme.bad_style()),
        (_, true) => ("✓", theme.ok_style()),
        (_, false) => (" ", theme.dim_style()),
    };
    let mut spans = vec![
        Span::styled(if selected { " ▸ " } else { "   " }, theme.accent_style()),
        Span::styled(format!("{mark} "), mark_style),
        Span::raw(fit(line.component.name(), columns.component)),
        Span::raw(" "),
    ];
    if columns.state > 0 {
        // What the row will be, against what the project has on disk, in the
        // words `components list` uses for the two settled states.
        let (text, style) = match (line.recorded, line.enabled) {
            (false, true) => ("adding", theme.ok_style()),
            (true, false) => ("removing", theme.bad_style()),
            (_, true) => ("enabled", theme.ok_style()),
            (_, false) => ("off", theme.dim_style()),
        };
        spans.push(Span::styled(fit(text, columns.state), style));
        spans.push(Span::raw(" "));
    }
    if columns.reach > 0 {
        let style = if line.enabled {
            theme.accent_style()
        } else {
            theme.dim_style()
        };
        spans.push(Span::styled(fit(&line.reach, columns.reach), style));
        spans.push(Span::raw(" "));
    }
    if columns.what > 0 {
        spans.push(Span::styled(
            fit(line.summary, columns.what),
            theme.dim_style(),
        ));
    }
    Line::from(spans)
}

/// The strip under the table: one line for the row under the cursor, or, when
/// there is something unsaved, a line for each thing saving would do.
fn draw_strip(frame: &mut Frame, area: Rect, app: &App, changes: &[Change], theme: &Theme) {
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
fn summary_lines<'a>(app: &App, width: usize, theme: &Theme) -> Vec<Line<'a>> {
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

/// The compose file `chaps sync` renders for a component.
///
/// chap-core's are upstream's own, which this CLI renders but does not author,
/// so they are named as the pair they are.
fn compose_of(component: Component) -> String {
    match component {
        Component::ChapCore => format!(
            "{} + {}",
            crate::project::BASE_COMPOSE,
            crate::project::CHAPS_COMPOSE
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

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    if let Some(message) = &app.message {
        let text = fit_soft(&format!(" {message}"), area.width as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, theme.warn_style()))),
            area,
        );
        return;
    }
    let counts = app.counts();
    let hints = keys::keybar(app.page, app.mode, counts.pending, !app.filter.is_empty());
    frame.render_widget(
        Paragraph::new(Line::from(keybar(&hints, area.width as usize, theme))),
        area,
    );
}

/// The host port dialog: what is being typed, what the row has now, why the
/// last thing typed was refused, and the keys that end it.
fn draw_port_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.page == Page::Components {
        draw_component_port_dialog(frame, area, app, theme);
        return;
    }
    let Some(row) = app.selected() else {
        return;
    };
    let now = match row.want {
        PortWant::None => "via chap-core".to_string(),
        PortWant::Auto => "auto".to_string(),
        PortWant::Exact(port) => format!("port {port}"),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(" port  ", theme.dim_style()),
            Span::styled("› ", theme.accent_style()),
            Span::styled(app.port_input.clone(), theme.accent_style()),
            Span::styled("_", theme.accent_style()),
        ]),
        Line::from(Span::styled(
            format!(
                " now: {now} · range {}-{} · api port {} is taken",
                app.port_range.0, app.port_range.1, app.api_port
            ),
            theme.dim_style(),
        )),
        // Always drawn, empty or not, so the keys below it never move.
        Line::from(Span::styled(
            match &app.port_error {
                Some(why) => format!(" {why}"),
                None => String::new(),
            },
            theme.bad_style(),
        )),
        Line::raw(""),
    ];
    let title = format!("Host port for {}", app.model(row).display_name);
    draw_dialog(
        frame,
        area,
        &title,
        lines,
        &keys::port_dialog_keys(app.page),
        theme,
    );
}

/// The same dialog for a component: no port range, because a component is not
/// in the model range, and no `auto`, because nothing allocates one for it.
fn draw_component_port_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let component = app.selected_component();
    let lines = vec![
        Line::from(vec![
            Span::styled(" port  ", theme.dim_style()),
            Span::styled("› ", theme.accent_style()),
            Span::styled(app.port_input.clone(), theme.accent_style()),
            Span::styled("_", theme.accent_style()),
        ]),
        Line::from(Span::styled(
            format!(
                " now: {} · api port {} is taken",
                app.component_reach(component),
                app.api_port
            ),
            theme.dim_style(),
        )),
        Line::from(Span::styled(
            match &app.port_error {
                Some(why) => format!(" {why}"),
                None => String::new(),
            },
            theme.bad_style(),
        )),
        Line::raw(""),
    ];
    let title = format!("Host port for {}", component.name());
    draw_dialog(
        frame,
        area,
        &title,
        lines,
        &keys::port_dialog_keys(app.page),
        theme,
    );
}

/// The channel dialog: the two channels, what each resolves to, and which one
/// the row follows today.
fn draw_channel_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(row) = app.selected() else {
        return;
    };
    let model = app.model(row);
    let mut lines = Vec::new();
    for (i, channel) in crate::tui::app::CHANNELS.iter().enumerate() {
        let version = match channel {
            Channel::Stable => &model.channels.stable,
            Channel::Latest => &model.channels.latest,
        };
        let selected = i == app.channel_cursor.min(1);
        let line = Line::from(vec![
            Span::styled(if selected { " ▸ " } else { "   " }, theme.accent_style()),
            Span::styled(
                if *channel == row.channel {
                    "✓ "
                } else {
                    "  "
                },
                theme.ok_style(),
            ),
            Span::raw(fit(channel_label(*channel), 8)),
            Span::styled(version.clone(), theme.dim_style()),
        ]);
        lines.push(match selected {
            true => line.style(theme.selection_style()),
            false => line,
        });
    }
    lines.push(Line::raw(""));
    let title = format!("Channel for {}", model.display_name);
    draw_dialog(
        frame,
        area,
        &title,
        lines,
        &keys::channel_dialog_keys(),
        theme,
    );
}

/// The DHIS2 version dialog: the versions on offer, which one is in force,
/// and whether chaps has a demo database for it.
fn draw_dhis2_version_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let current = &app.components.dhis2.image_tag;
    let mut lines = Vec::new();
    for (i, version) in app.dhis2_versions.iter().enumerate() {
        let selected = i == app.dhis2_version_cursor;
        let seed = match crate::components::dhis2_seed_dump(version) {
            Some(_) => "demo database",
            None => "empty database",
        };
        let line = Line::from(vec![
            Span::styled(if selected { " ▸ " } else { "   " }, theme.accent_style()),
            Span::styled(
                if version == current { "✓ " } else { "  " },
                theme.ok_style(),
            ),
            Span::raw(fit(version, 10)),
            Span::styled(seed.to_string(), theme.dim_style()),
        ]);
        lines.push(match selected {
            true => line.style(theme.selection_style()),
            false => line,
        });
    }
    lines.push(Line::raw(""));
    draw_dialog(
        frame,
        area,
        "DHIS2 version",
        lines,
        &keys::dhis2_version_dialog_keys(),
        theme,
    );
}

/// A centred dialog: a rounded box with its own key line along the bottom.
fn draw_dialog(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    mut lines: Vec<Line<'static>>,
    hints: &[Hint],
    theme: &Theme,
) {
    // Wide enough for its own key line, its title and the widest thing it
    // says - a refusal is the reason the box is up, so it is not the thing to
    // cut - with a column to spare on each side, and never wider than the
    // terminal.
    let widest = lines.iter().map(width_of_line).max().unwrap_or(0);
    let width = ((bar_width(&hints.iter().collect::<Vec<&Hint>>()) + 4) as u16)
        .max(title.chars().count() as u16 + 6)
        .max(widest as u16 + 3)
        .min(area.width.saturating_sub(4))
        .max(1);
    let inner = width.saturating_sub(2) as usize;
    lines.push(Line::from(keybar(hints, inner, theme)));
    let height = (lines.len() as u16 + 2).min(area.height).max(1);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(overlay(theme, title)), popup);
}

/// The key bar: `[key] what`, the key in the accent so the line reads as keys
/// first, and the entries with the least to say dropped when it does not fit.
fn keybar<'a>(hints: &[Hint], width: usize, theme: &Theme) -> Vec<Span<'a>> {
    let mut kept: Vec<&Hint> = hints.iter().collect();
    // One leading space, two between entries.
    while kept.len() > 1 && 1 + bar_width(&kept) > width {
        let Some(worst) = kept
            .iter()
            .enumerate()
            .min_by_key(|(i, hint)| (hint.rank, *i))
            .map(|(i, _)| i)
        else {
            break;
        };
        kept.remove(worst);
    }

    let mut spans = vec![Span::raw(" ")];
    for (i, hint) in kept.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        if hint.chip {
            spans.push(Span::styled(
                format!(" [{}] {} ", hint.key, hint.what),
                theme.chip_style(),
            ));
            continue;
        }
        spans.push(Span::styled("[", theme.dim_style()));
        // The key itself is the only bold thing on the bar, so the line reads
        // as keys with words after them rather than as a sentence.
        spans.push(Span::styled(
            hint.key.to_string(),
            theme.accent_style().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("] ", theme.dim_style()));
        spans.push(Span::styled(hint.what.clone(), theme.dim_style()));
    }
    spans
}

/// How wide a key bar would be, separators and all.
fn bar_width(hints: &[&Hint]) -> usize {
    let entries: usize = hints
        .iter()
        .map(|hint| {
            // `[key] what`, or ` [key] what ` when it is a chip.
            let plain = hint.key.chars().count() + hint.what.chars().count() + 3;
            if hint.chip { plain + 2 } else { plain }
        })
        .sum();
    entries + hints.len().saturating_sub(1) * 2
}

/// The details overlay: everything the catalogue and the project know about
/// the row under the cursor, scrolled by `j` and `k`.
fn draw_info(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let row = match app.page {
        Page::Models => match app.selected() {
            Some(row) => Some(row),
            None => return,
        },
        Page::Components => None,
    };

    let width = area.width.saturating_sub(6).clamp(1, 86);
    // One column of padding inside the border, as the list has.
    let inner_w = width.saturating_sub(3) as usize;
    let body = match row {
        Some(row) => info_lines(app, row, inner_w, theme),
        None => component_info_lines(app, inner_w, theme),
    };
    let lines: Vec<Line> = body
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect();
    // As tall as it needs to be, with the list still visible around it; what
    // does not fit is what `j` and `k` scroll.
    let height = (lines.len() as u16 + 2)
        .min(area.height.saturating_sub(6))
        .max(3.min(area.height));
    let popup = centered(area, width, height);
    let inner_h = popup.height.saturating_sub(2) as usize;
    app.info_max.set(lines.len().saturating_sub(inner_h));
    let scroll = app.info_scroll.min(app.info_max.get()) as u16;

    let title_w = popup.width.saturating_sub(2) as usize;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.accent_style())
        .title(match row {
            Some(row) => info_title(app, row, title_w, theme),
            None => component_info_title(app, title_w, theme),
        });

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        popup,
    );
}

/// The overlay's title: what the model is called, what `models.yaml` calls
/// it, and whether this deployment runs it.
///
/// The state is right-aligned and always fits; the id gives way first,
/// because the display name in front of it already says which model this is.
/// The assessed status is not up here - it has a row of its own.
fn info_title<'a>(app: &App, row: &Row, width: usize, theme: &Theme) -> Line<'a> {
    let model = app.model(row);
    let (state, style) = match app.recorded(row) {
        Some(recorded) => (
            match recorded.host_port {
                Some(port) => format!("enabled on port {port}"),
                None => "enabled, via chap-core".to_string(),
            },
            theme.ok_style(),
        ),
        None => ("not enabled here".to_string(), theme.dim_style()),
    };
    let head = vec![
        Span::raw(" "),
        Span::styled(model.display_name.clone(), theme.accent_style()),
        Span::raw("  "),
        Span::styled(model.id.clone(), theme.dim_style()),
    ];
    Line::from(with_kept_tail(
        head,
        Span::styled(format!("{state} "), style),
        width,
    ))
}

/// The component overlay's title: the component, and whether this deployment
/// has it.
///
/// What the deployment has, not what the session wants: the model overlay's
/// title reads off the project the same way, and the `state` row inside is
/// where a pending change is said.
fn component_info_title<'a>(app: &App, width: usize, theme: &Theme) -> Line<'a> {
    let component = app.selected_component();
    let enabled = app.initial_components.is_enabled(component);
    let (state, style) = match enabled {
        true => ("enabled here".to_string(), theme.ok_style()),
        false => ("not enabled here".to_string(), theme.dim_style()),
    };
    let head = vec![
        Span::raw(" "),
        Span::styled(component.name().to_string(), theme.accent_style()),
    ];
    Line::from(with_kept_tail(
        head,
        Span::styled(format!("{state} "), style),
        width,
    ))
}

/// Everything the browser knows about a component: what it is, what `sync`
/// renders for it, where its data lives, and - for a component with a config
/// file of its own - that file plus the settings this page deliberately leaves
/// to `.chaps/`.
fn component_info_lines<'a>(app: &App, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let component = app.selected_component();
    let enabled = app.components.is_enabled(component);
    let mut lines: Vec<Line> = Vec::new();

    for line in wrap(component.summary(), width) {
        lines.push(Line::raw(line));
    }
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![
        Span::styled(fit("state", LABEL_WIDTH), theme.label_style()),
        Span::styled(
            match (app.initial_components.is_enabled(component), enabled) {
                (false, true) => "off · this session would add it".to_string(),
                (true, false) => "enabled · this session would remove it".to_string(),
                (_, true) => "enabled".to_string(),
                (_, false) => "off".to_string(),
            },
            match enabled {
                true => theme.ok_style(),
                false => theme.dim_style(),
            },
        ),
    ]));
    lines.push(field(theme, "reach", &app.component_reach(component)));
    lines.extend(wrapped_field(
        theme,
        "compose",
        &format!(
            "{} · rendered from .chaps/{} by `chaps sync`",
            compose_of(component),
            crate::components::COMPONENTS_FILE
        ),
        width,
    ));
    lines.extend(wrapped_field(
        theme,
        "volume",
        &volume_field(component.volumes()),
        width,
    ));
    if component == Component::Ocs {
        lines.extend(wrapped_field(
            theme,
            "config",
            &format!(
                "{}/{} · yours to edit, and chaps never rewrites it",
                crate::components::OCS_DIR,
                crate::components::OCS_CONFIG_FILE
            ),
            width,
        ));
        lines.push(Line::raw(""));
        // The two OCS settings the browser has no dialog for, named here so
        // this overlay is where someone learns they exist.
        lines.extend(not_on_this_page(
            theme,
            width,
            &[
                (
                    "`chaps components enable ocs --base-url URL`",
                    "the public origin OCS builds its STAC and openEO links from",
                ),
                (
                    "`chaps components enable ocs --read-only`",
                    "refuse ingestion over HTTP (--read-write allows it again)",
                ),
            ],
        ));
    }
    if component == Component::Dhis2 {
        lines.extend(wrapped_field(
            theme,
            "config",
            &format!(
                "{}/{} · yours to edit, and DHIS2 will not start without it",
                crate::components::DHIS2_DIR,
                crate::components::DHIS2_CONFIG_FILE
            ),
            width,
        ));
        lines.extend(wrapped_field(
            theme,
            "seed",
            &dhis2_seed_field(&app.components),
            width,
        ));
        // The `volume` field above names all three, so this says which of them
        // is not data: the one an operator would otherwise try to keep.
        lines.extend(wrapped_field(
            theme,
            "cache",
            &format!(
                "{} holds the downloaded dump rather than data; the dhis2-dump one-shot \
                 fetches it again when the volume is empty",
                crate::compose::render::DHIS2_DUMP_VOLUME
            ),
            width,
        ));
        lines.extend(wrapped_field(
            theme,
            "first start",
            "minutes, not seconds: DHIS2 migrates its schema on the way up, and \
             `chaps logs dhis2` is where that shows",
            width,
        ));
        lines.push(Line::raw(""));
        // The seed and the image tag, which no flag moves after `init`: they are
        // an edit to `.chaps/components.yaml` and a sync, and this overlay is
        // where someone finds that out rather than in the YAML.
        lines.extend(not_on_this_page(
            theme,
            width,
            &[
                (
                    "`seed:` in .chaps/components.yaml, then `chaps sync`",
                    "the dump a database being created is restored from: default, none, \
                     a URL, or a path in the deployment directory",
                ),
                (
                    "`image_tag:` in .chaps/components.yaml, then `chaps sync`",
                    &format!(
                        "the DHIS2 version, {} here; it migrates a schema forward only, \
                         so run `chaps backup` first",
                        app.components.dhis2.image_tag
                    ),
                ),
            ],
        ));
    }
    if component == Component::ChapCore {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "its host port is the API port, which lives in .chaps/project.yaml",
            theme.dim_style(),
        )));
    }
    lines
}

/// The closing block of a component's overlay: the settings the browser has no
/// dialog for, each with what it does.
///
/// Named rather than left out, so this overlay is where someone learns they
/// exist. Each `how` carries its own backticks, because one component's are
/// whole commands and another's are a key in `.chaps/components.yaml` followed
/// by a sync, and the reader has to see which of the two they are looking at.
fn not_on_this_page<'a>(theme: &Theme, width: usize, settings: &[(&str, &str)]) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(Span::styled(
        "not on this page:",
        theme.label_style(),
    ))];
    for (how, what) in settings {
        lines.push(Line::from(Span::styled(
            format!("  {how}"),
            theme.accent_style(),
        )));
        for chunk in wrap(what, width.saturating_sub(4).max(1)) {
            lines.push(Line::from(Span::styled(
                format!("    {chunk}"),
                theme.dim_style(),
            )));
        }
    }
    lines
}

/// The `seed` field of the DHIS2 overlay: what the first start restores, and
/// that a restore only ever happens to a database being created.
///
/// Four answers rather than the three `.chaps/components.yaml` records, because
/// a `default` on a minor line chaps publishes no dump for starts empty as well,
/// and for a reason worth saying: it otherwise looks exactly like `none`, and
/// nothing else on this page would explain it.
fn dhis2_seed_field(components: &crate::components::Components) -> String {
    let volume = crate::compose::render::DHIS2_DB_VOLUME;
    if components.dhis2_seed_is_unknown() {
        return format!(
            "default · chaps knows no dump for {}, so {volume} starts empty",
            crate::components::dhis2_minor(&components.dhis2.image_tag)
        );
    }
    match components.dhis2_seed_source() {
        Some(source) => {
            format!("{source} · restored once, into the database the first `chaps up` creates")
        }
        None => format!("none · {volume} starts empty and DHIS2 migrates a new database into it"),
    }
}

/// The `volume` field of the component overlay: every volume the component
/// keeps, or what to run for a component that keeps none of its own.
///
/// All of them on the one field, comma separated, since the field is what the
/// overlay exists for - saying where a component's data lives - and a component
/// with two volumes has it in two places.
fn volume_field(volumes: &[&str]) -> String {
    match volumes {
        [] => "none of its own · `chaps down --volumes` removes this deployment's".to_string(),
        volumes => format!(
            "{} · kept when the component is disabled",
            volumes.join(", ")
        ),
    }
}

/// Every field the overlay lists, in the order it lists them.
fn info_lines<'a>(app: &App, row: &Row, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let model = app.model(row);
    let mut lines: Vec<Line> = Vec::new();

    // What the model does comes first: everything under it is detail about a
    // model the reader has already decided to look at.
    for line in wrap(&model.summary, width) {
        lines.push(Line::raw(line));
    }
    lines.push(Line::raw(""));

    // What this row is pinned to, how far that pin is trusted, and which
    // channels point at it: one fact, so one row.
    let pinned = match app.recorded(row) {
        Some(recorded) => Some((
            version_and_tag(&recorded.version, &recorded.image_tag),
            recorded.version.clone(),
        )),
        None => app
            .resolved(row)
            .map(|v| (version_and_tag(&v.version, &v.image_tag), v.version.clone())),
    };
    match pinned {
        Some((shown, version)) => {
            let entry = model.versions.iter().find(|v| v.version == version);
            lines.push(version_line(model, "version", &shown, entry, theme));
            // The rest of the published history, when there is any.
            if model.versions.len() > 1 {
                for other in model.versions.iter().filter(|v| v.version != version) {
                    lines.push(version_line(
                        model,
                        "",
                        &version_and_tag(&other.version, &other.image_tag),
                        Some(other),
                        theme,
                    ));
                }
            }
        }
        None => lines.push(field(
            theme,
            "version",
            &format!(
                "unresolved: channel {} has no version",
                channel_label(row.channel)
            ),
        )),
    }

    let mut author = model.attribution.author.clone();
    if let Some(organization) = &model.attribution.organization {
        author.push_str(&format!(" · {organization}"));
    }
    if let Some(contact) = &model.attribution.contact {
        author.push_str(&format!(" · {contact}"));
    }
    lines.extend(wrapped_field(theme, "author", &author, width));

    // The assessment wraps rather than being cut: half of "not intended for
    // use" is worse than no sentence at all.
    let assessment = format!(
        "{}, {}",
        model.assessed_status.colour(),
        model.assessed_status.describe()
    );
    let room = width.saturating_sub(LABEL_WIDTH + 2).max(1);
    for (i, chunk) in wrap(&assessment, room).into_iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                fit(if i == 0 { "status" } else { "" }, LABEL_WIDTH),
                theme.label_style(),
            ),
            match i {
                0 => Span::styled("● ", theme.status_style(model.assessed_status)),
                _ => Span::raw("  "),
            },
            Span::raw(chunk),
        ]));
    }

    lines.push(field(
        theme,
        "period",
        &format!(
            "{} · {}",
            model.compatibility.period_types.join(", "),
            horizon(app, row)
        ),
    ));
    // The marketplace schema carries no target: chap-core forecasts disease
    // cases for every model in the catalogue, so that is what this says.
    lines.push(field(theme, "target", "disease cases"));

    let mut covariates = format!(
        "{} · defaults {}",
        list_or_dash(&model.covariates.required),
        list_or_dash(&model.covariates.defaults)
    );
    if model.covariates.allow_free_additional {
        covariates.push_str(" · free extras allowed");
    }
    if model.compatibility.requires_geo {
        covariates.push_str(" · geometry required");
    }
    lines.extend(wrapped_field(theme, "covariates", &covariates, width));

    // Everything below is about this deployment rather than the model.
    lines.push(Line::raw(""));

    // A template is not a forecasting model, and the overlay says so in a
    // colour that is neither "good" nor "bad", just different.
    if model.is_template() {
        lines.push(Line::from(vec![
            Span::styled(fit("kind", LABEL_WIDTH), theme.label_style()),
            Span::styled(
                "template (scaffolding, not a forecasting model)",
                theme.template_style(),
            ),
        ]));
    } else if model.manual {
        lines.push(Line::from(vec![
            Span::styled(fit("kind", LABEL_WIDTH), theme.label_style()),
            Span::styled("manual (added with `chaps models add`)", theme.dim_style()),
        ]));
    }

    let reach = match app.recorded(row).and_then(|r| r.host_port) {
        Some(port) => format!("http://localhost:{port}"),
        None => "via chap-core (through chap-core's /run/ proxy)".to_string(),
    };
    lines.push(field(theme, "reach", &reach));

    let image = match app.resolved(row) {
        Some(version) => crate::compose::image_ref(&model.source.image, &version.image_tag),
        None => model.source.image.clone(),
    };
    lines.extend(wrapped_field(theme, "image", &image, width));

    let runtime = if model.needs_amd64() {
        format!("{} (amd64 only)", model.source.runtime_image)
    } else {
        model.source.runtime_image.clone()
    };
    lines.extend(wrapped_field(theme, "runtime", &runtime, width));

    if let Some(recorded) = app.recorded(row) {
        lines.push(field(theme, "data dir", &recorded.data_dir));
        lines.push(field(
            theme,
            "user",
            &format!("{} ({})", recorded.user, recorded.user_from.label()),
        ));
    }

    lines.push(Line::raw(""));

    if !model.maintainers.is_empty() {
        lines.extend(wrapped_field(
            theme,
            "maintainers",
            &model.maintainers.join(", "),
            width,
        ));
    }
    lines.push(Line::from(vec![
        Span::styled(fit("repository", LABEL_WIDTH), theme.label_style()),
        Span::styled(model.source.repository.clone(), theme.accent_style()),
    ]));
    if let Some(citation) = &model.attribution.citation {
        lines.extend(wrapped_field(theme, "citation", citation, width));
    }
    lines
}

/// One published version: what it is, how far it is trusted, and the channels
/// pointing at it.
fn version_line<'a>(
    model: &crate::registry::Model,
    label: &str,
    shown: &str,
    entry: Option<&Version>,
    theme: &Theme,
) -> Line<'a> {
    let mut spans = vec![
        Span::styled(fit(label, LABEL_WIDTH), theme.label_style()),
        Span::raw(shown.to_string()),
    ];
    let Some(entry) = entry else {
        return Line::from(spans);
    };
    spans.push(Span::styled(" · ", theme.dim_style()));
    spans.push(Span::styled(
        version_status(entry),
        theme.version_style(entry.status),
    ));
    let mut channels: Vec<&str> = Vec::new();
    if entry.version == model.channels.stable {
        channels.push("stable");
    }
    if entry.version == model.channels.latest {
        channels.push("latest");
    }
    if !channels.is_empty() {
        spans.push(Span::styled(" · channels ", theme.dim_style()));
        spans.push(Span::styled(channels.join(", "), theme.accent_style()));
    }
    Line::from(spans)
}

/// The command palette: a filter line, what it matched, and every command it
/// could have matched.
fn draw_palette(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let matches = app.palette_matches();
    let total = app.commands().len();
    let width = area.width.saturating_sub(4).clamp(1, 72);

    let inner_w = width.saturating_sub(2) as usize;
    let strip = wrap(
        &app.commands()
            .iter()
            .map(|c| c.short)
            .collect::<Vec<&str>>()
            .join(" · "),
        inner_w.saturating_sub(1),
    );
    let strip: Vec<String> = strip.into_iter().take(3).collect();
    let rows = matches.len().max(1);
    let height = (2 + 1 + 1 + rows + 1 + strip.len()) as u16;
    let popup = centered(area, width, height.min(area.height.max(1)));

    let mut lines = vec![prompt_line(app, matches.len(), total, inner_w, theme)];
    lines.push(rule_line(inner_w, theme));
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no command matches",
            theme.dim_style(),
        )));
    }
    for (i, command) in matches.iter().enumerate() {
        lines.push(command_line(
            command,
            i == app.palette_cursor,
            inner_w,
            theme,
        ));
    }
    lines.push(rule_line(inner_w, theme));
    for line in strip {
        lines.push(Line::from(Span::styled(
            format!(" {line}"),
            theme.dim_style(),
        )));
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(overlay(theme, "Commands")),
        popup,
    );
}

/// `› po_` on the left, how much of the list is left on the right.
fn prompt_line<'a>(
    app: &App,
    matched: usize,
    total: usize,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let head = vec![
        Span::styled(" › ", theme.accent_style()),
        Span::styled(app.palette_query.clone(), theme.label_style()),
        Span::styled("_", theme.accent_style()),
    ];
    Line::from(with_tail(
        head,
        Span::styled(format!("{matched} of {total} commands "), theme.dim_style()),
        width,
    ))
}

/// One command: the cursor, the label with the typed text picked out, and the
/// key that does the same thing from the list.
fn command_line<'a>(command: &Command, selected: bool, width: usize, theme: &Theme) -> Line<'a> {
    let mut spans = vec![Span::styled(
        if selected { " ▸ " } else { "   " },
        theme.accent_style(),
    )];
    match command.hit {
        Some((from, to)) => {
            let chars: Vec<char> = command.label.chars().collect();
            spans.push(Span::raw(chars[..from].iter().collect::<String>()));
            spans.push(Span::styled(
                chars[from..to].iter().collect::<String>(),
                theme.accent_style(),
            ));
            spans.push(Span::raw(chars[to..].iter().collect::<String>()));
        }
        None => spans.push(Span::raw(command.label.clone())),
    }
    // The trailing space keeps the key off the border, and the padding that
    // puts it there is what makes the highlight cover the whole row.
    let line = Line::from(with_tail(
        spans,
        Span::styled(format!("{} ", command.key), theme.dim_style()),
        width,
    ));
    if selected {
        line.style(theme.selection_style())
    } else {
        line
    }
}

fn rule_line<'a>(width: usize, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled("─".repeat(width), theme.dim_style()))
}

fn draw_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let entries = keys::help_entries();
    let width = 56u16;
    let height = entries.len() as u16 + 2;
    let popup = centered(area, width, height);

    let lines: Vec<Line> = entries
        .iter()
        .map(|&(key, what)| {
            Line::from(vec![
                Span::styled(format!(" {}", fit(key, 20)), theme.accent_style()),
                Span::styled(what.to_string(), theme.dim_style()),
            ])
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(overlay(theme, "Keys")), popup);
}

fn draw_confirm(frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered(area, 46, 4);
    let lines = vec![
        Line::from(Span::styled(
            " There are unsaved changes.",
            theme.warn_style(),
        )),
        Line::from(vec![
            Span::raw(" Quit anyway? "),
            Span::styled("y", theme.bad_style()),
            Span::raw(" / "),
            Span::styled("n", theme.ok_style()),
        ]),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(overlay(theme, "Quit")), popup);
}

/// A pane: rounded border, a title that follows the border's colour.
fn pane<'a>(theme: &Theme, title: &str, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(Span::styled(
            format!(" {title} "),
            theme.pane_title_style(focused),
        ))
}

/// An overlay: the same shape as a pane, always in the accent.
fn overlay<'a>(theme: &Theme, title: &str) -> Block<'a> {
    pane(theme, title, true)
}

/// A centred rectangle that never leaves `area`, whatever the terminal size.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn field<'a>(theme: &Theme, label: &str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(fit(label, LABEL_WIDTH), theme.label_style()),
        Span::raw(value.to_string()),
    ])
}

/// [`field`] for a value that can be longer than the box: it wraps under its
/// own label instead of being cut off at the border.
fn wrapped_field<'a>(theme: &Theme, label: &str, value: &str, width: usize) -> Vec<Line<'a>> {
    let room = width.saturating_sub(LABEL_WIDTH).max(1);
    let chunks = wrap(value, room);
    if chunks.is_empty() {
        return vec![field(theme, label, "")];
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            Line::from(vec![
                Span::styled(
                    fit(if i == 0 { label } else { "" }, LABEL_WIDTH),
                    theme.label_style(),
                ),
                Span::raw(chunk),
            ])
        })
        .collect()
}

/// Push `tail` to the right-hand edge of `width`, or leave it off when the
/// head already fills the line.
fn with_tail<'a>(mut head: Vec<Span<'a>>, tail: Span<'a>, width: usize) -> Vec<Span<'a>> {
    let used = width_of(&head);
    let tail_width = tail.content.chars().count();
    if tail_width == 0 {
        truncate(&mut head, width);
        return head;
    }
    if used + tail_width + 2 > width {
        truncate(&mut head, width);
        return head;
    }
    head.push(Span::raw(" ".repeat(width - used - tail_width)));
    head.push(tail);
    head
}

/// [`with_tail`] for a tail that has to stay: the head is cut to make room
/// for it, because a hint that only shows on a wide terminal is no hint.
fn with_kept_tail<'a>(mut head: Vec<Span<'a>>, tail: Span<'a>, width: usize) -> Vec<Span<'a>> {
    let tail_width = tail.content.chars().count();
    if tail_width == 0 || tail_width + 2 > width {
        truncate(&mut head, width);
        return head;
    }
    truncate(&mut head, width - tail_width - 2);
    let used = width_of(&head);
    head.push(Span::raw(" ".repeat(width - used - tail_width)));
    head.push(tail);
    head
}

/// Cut a run of spans down to `width` columns, marking the cut with `~` so
/// the line never pretends it said everything.
fn truncate(spans: &mut Vec<Span>, width: usize) {
    if width_of(spans) <= width {
        return;
    }
    if width == 0 {
        spans.clear();
        return;
    }
    let mut used = 0usize;
    for i in 0..spans.len() {
        let len = spans[i].content.chars().count();
        if used + len < width {
            used += len;
            continue;
        }
        let mut cut: String = spans[i].content.chars().take(width - used - 1).collect();
        cut.push('~');
        spans[i].content = cut.into();
        spans.truncate(i + 1);
        return;
    }
}

/// How many columns a whole line occupies.
fn width_of_line(line: &Line) -> usize {
    width_of(&line.spans)
}

/// How many columns a run of spans occupies.
fn width_of(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

fn list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn horizon(app: &App, row: &Row) -> String {
    let c = &app.model(row).compatibility;
    format!(
        "horizon {} to {} periods",
        c.min_prediction_periods, c.max_prediction_periods
    )
}

/// `1.0.3 (sha-24d58c0)`, or just the tag when the two are the same thing, as
/// they are for a manually added model.
fn version_and_tag(version: &str, tag: &str) -> String {
    if version == tag {
        tag.to_string()
    } else {
        format!("{version} ({tag})")
    }
}

/// Pad or truncate to exactly `width` characters.
fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count == width {
        return text.to_string();
    }
    if count < width {
        let mut out = text.to_string();
        out.push_str(&" ".repeat(width - count));
        return out;
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('~');
    out
}

/// [`fit`] without the padding: shorten what is too long, leave the rest.
fn fit_soft(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    fit(text, width)
}

/// Break text into lines of at most `width` columns, on spaces where there is
/// one and mid-word where there is not.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let line_len = line.chars().count();
        if line.is_empty() {
            line = word.to_string();
        } else if line_len + 1 + word_len <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
        while line.chars().count() > width {
            let head: String = line.chars().take(width).collect();
            let tail: String = line.chars().skip(width).collect();
            lines.push(head);
            line = tail;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn channel_label(channel: Channel) -> &'static str {
    channel.as_str()
}

fn version_status(version: &Version) -> &'static str {
    match version.status {
        VersionStatus::Verified => "verified",
        VersionStatus::Unstable => "unstable",
        VersionStatus::Deprecated => "deprecated",
        VersionStatus::Yanked => "yanked",
    }
}

#[cfg(test)]
mod tests;
