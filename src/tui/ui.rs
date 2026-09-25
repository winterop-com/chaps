//! TUI rendering: title bar, the model table, the summary strip, key bar and
//! the overlays.
//!
//! Owned by agent C.
//!
//! Drawing never mutates the [`App`] except for one thing it alone can know:
//! how far the details overlay can scroll, which depends on the size of the
//! terminal. Everything else on screen is derived from the state the reducer
//! produced, and every colour comes from the [`Theme`]. The layout degrades on
//! narrow terminals by dropping columns rather than wrapping or panicking.

use crate::registry::{Channel, Provenance, Version, VersionStatus};
use crate::tui::app::{App, Change, ChangeKind, Command, Mode, PortWant, Row};
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
        Span::styled("models", theme.label_style()),
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
    let totals = Span::styled(
        format!("{} models · {} enabled · ", counts.total, counts.enabled),
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

    let columns = columns(inner.width as usize, kind_column(app));
    draw_head(frame, head, &columns, theme);
    draw_rule(frame, rule_top, theme);
    draw_list(frame, list, app, &columns, theme);
    draw_rule(frame, rule_bottom, theme);
    draw_strip(frame, strip, app, &changes, theme);
}

/// `Marketplace`, plus what a filter left of it.
fn pane_title(app: &App) -> String {
    if app.filter.is_empty() {
        return "Marketplace".to_string();
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

/// One table row: `▸ ✓ Display Name   id   ● status   1.0.2   internal`.
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

/// The strip under the table: one line for the row under the cursor, or, when
/// there is something unsaved, a line for each thing saving would do.
fn draw_strip(frame: &mut Frame, area: Rect, app: &App, changes: &[Change], theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // The strip is indented the way the rows are, so the two read as one box.
    let width = area.width.saturating_sub(1) as usize;
    let lines = if changes.is_empty() {
        summary_lines(app, width, theme)
    } else {
        change_lines(changes, width, theme)
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
    let hints = keys::keybar(app.mode, counts.pending, !app.filter.is_empty());
    frame.render_widget(
        Paragraph::new(Line::from(keybar(&hints, area.width as usize, theme))),
        area,
    );
}

/// The host port dialog: what is being typed, what the row has now, why the
/// last thing typed was refused, and the keys that end it.
fn draw_port_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
    draw_dialog(frame, area, &title, lines, &keys::port_dialog_keys(), theme);
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

/// A centred dialog: a rounded box with its own key line along the bottom.
fn draw_dialog(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    mut lines: Vec<Line<'static>>,
    hints: &[Hint],
    theme: &Theme,
) {
    // Wide enough for its own key line, with a column to spare on each side,
    // and never wider than the terminal.
    let width = ((bar_width(&hints.iter().collect::<Vec<&Hint>>()) + 4) as u16)
        .max(title.chars().count() as u16 + 6)
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
    let Some(row) = app.selected() else {
        return;
    };

    let width = area.width.saturating_sub(6).clamp(1, 86);
    // One column of padding inside the border, as the list has.
    let inner_w = width.saturating_sub(3) as usize;
    let lines: Vec<Line> = info_lines(app, row, inner_w, theme)
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

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.accent_style())
        .title(info_title(
            app,
            row,
            popup.width.saturating_sub(2) as usize,
            theme,
        ));

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

/// `1.0.2 (sha-8d4a7ea)`, or just the tag when the two are the same thing, as
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
mod tests {
    use super::*;
    use crate::project::ProjectState;
    use crate::registry::{Registry, load_embedded};
    use crate::tui::app::Action;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const EWARS: &str = "chapkit_ewars_model";

    fn registry() -> Registry {
        load_embedded().expect("embedded snapshot parses")
    }

    fn theme() -> Theme {
        Theme::default()
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        render_with(app, width, height, &theme())
    }

    fn render_with(app: &App, width: u16, height: u16, theme: &Theme) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend starts");
        terminal
            .draw(|frame| draw(frame, app, theme))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map(|c| c.symbol().to_string())
                            .unwrap_or_default()
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The line of the rendered screen that contains `needle`.
    fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
        screen
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line holds {needle}:\n{screen}"))
    }

    /// Which column of a rendered line `needle` starts in. Counted in
    /// characters, because `│` and `▸` are three bytes each and byte offsets
    /// would make two columns that line up look as if they did not.
    fn column_of(line: &str, needle: &str) -> usize {
        let byte = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} is not on {line}"));
        line[..byte].chars().count()
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_normal_terminal_shows_the_title_the_table_and_the_summary() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 120, 40);
        assert!(screen.contains("chaps · models"));
        assert!(screen.contains("registry: embedded"));
        assert!(screen.contains(&format!(
            "{} models · 0 enabled · 0 pending",
            registry.models.len()
        )));
        assert!(screen.contains("Marketplace"));
        assert!(
            !screen.contains("Details"),
            "the details pane is an overlay now, not a second column"
        );
        // Rounded corners, not square ones.
        assert!(screen.contains('╭') && screen.contains('╯'), "{screen}");
        assert!(screen.contains("MODEL") && screen.contains("PORT"));
        assert!(screen.contains("CHAP-EWARS"));
        assert!(screen.contains("chapkit_ewars_model"));
        assert!(
            screen.contains("● limited data"),
            "the dot carries the colour, the word its meaning:\n{screen}"
        );
        assert!(
            !screen.contains("● orange"),
            "a bare colour word says nothing:\n{screen}"
        );
        assert!(screen.contains("[space] toggle"), "{screen}");
        // The summary strip stands in for the pane that used to be there.
        assert!(screen.contains("i for details"), "{screen}");
        assert!(screen.contains("requires population"), "{screen}");
        assert!(
            !screen.contains("Bayesian hierarchical"),
            "the summary text is behind `i`, not on the strip:\n{screen}"
        );
    }

    /// The headings mean nothing if they sit over the wrong columns.
    #[test]
    fn the_column_headings_line_up_with_the_rows() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 120, 40);
        let head = line_with(&screen, "MODEL");
        let row = line_with(&screen, "CHAP-EWARS  ");

        assert_eq!(column_of(head, "MODEL"), column_of(row, "CHAP-EWARS"));
        assert_eq!(column_of(head, "ID"), column_of(row, "chapkit_ewars_model"));
        assert_eq!(column_of(head, "STATUS"), column_of(row, "●"));
        assert_eq!(column_of(head, "VERSION"), column_of(row, "1.0.2"));
        // And the version has lost the `v` the old list put in front of it.
        assert!(!row.contains("v1.0.2"), "{row}");

        let enabled = state_with_ewars(&registry, Some(5001));
        let app = App::new(&registry, &enabled);
        let screen = render(&app, 120, 40);
        let head = line_with(&screen, "MODEL");
        let row = line_with(&screen, "✓ CHAP-EWARS");
        assert_eq!(column_of(head, "PORT"), column_of(row, "5001"));
    }

    /// A project state with `chapkit_ewars_model` enabled on `host_port`.
    fn state_with_ewars(registry: &Registry, host_port: Option<u16>) -> ProjectState {
        let mut state = ProjectState::default();
        let model = registry.get(EWARS).unwrap();
        state.models.insert(
            model.id.clone(),
            crate::project::EnabledModel {
                service_id: model.service_id.clone(),
                image: model.source.image.clone(),
                image_tag: model
                    .version(&model.channels.stable)
                    .expect("stable resolves")
                    .image_tag
                    .clone(),
                version: model.channels.stable.clone(),
                channel: Some(Channel::Stable),
                host_port,
                data_dir: "/app/data".into(),
                user: "chapkit:chapkit".into(),
                user_from: Default::default(),
                platform: Some("linux/amd64".into()),
                compose_file: "compose.chapkit-ewars-model.yml".into(),
            },
        );
        state
    }

    #[test]
    fn enabled_rows_are_ticked_and_show_their_port() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let app = App::new(&registry, &state);
        let screen = render(&app, 120, 40);
        assert!(screen.contains('✓'), "{screen}");
        assert!(screen.contains("5001"));
        // The strip says the same thing in words.
        assert!(screen.contains("enabled on port 5001"), "{screen}");
        assert!(
            !screen.contains("user chapkit:chapkit"),
            "the service user is behind `i`, not on the strip:\n{screen}"
        );
    }

    /// The port column is about how the model is reached, not about whether
    /// it is enabled - the tick already says that.
    #[test]
    fn an_enabled_model_with_no_host_port_is_reached_through_chap_core() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        let screen = render(&app, 120, 40);
        assert!(screen.contains('✓'));
        assert!(screen.contains("via chap-core"), "{screen}");
        assert!(
            screen.contains("enabled, via chap-core"),
            "and so does the strip"
        );
        assert!(!screen.contains("5001"), "no host port is published");
        assert!(!screen.contains("internal"), "{screen}");

        // The dialog is visible before saving, and so is what it took: the
        // port itself is only picked when the selection is applied.
        app.reduce(Action::PortPrompt);
        let prompt = render(&app, 120, 40);
        assert!(prompt.contains("Host port for CHAP-EWARS"), "{prompt}");
        assert!(prompt.contains("port  › auto_"), "{prompt}");
        assert!(prompt.contains("[enter] apply"), "{prompt}");
        app.reduce(Action::PortApply);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("auto"), "{screen}");
    }

    #[test]
    fn a_pending_change_is_marked_before_it_is_saved() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let mut app = App::new(&registry, &state);
        let columns = columns(118, false);

        // The first row is the one the project already runs; turning it off is
        // a removal, not an absence.
        app.reduce(Action::Toggle);
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, &columns, true, &theme());
        assert_eq!(line.spans[1].content, "- ");
        assert_eq!(line.spans[1].style, theme().bad_style());
        assert!(line_text(&line).contains("will be disabled"), "{line:?}");

        // And back on again is no change at all.
        app.reduce(Action::Toggle);
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, &columns, true, &theme());
        assert_eq!(line.spans[1].content, "✓ ");
        assert!(
            line_text(&line).contains("via chap-core"),
            "a row toggled off and on again comes back without its port"
        );

        // A row the project never had reads as an addition.
        let mut fresh = App::new(&registry, &ProjectState::default());
        fresh.reduce(Action::Toggle);
        let row = fresh.selected().expect("a row is selected");
        let line = row_line(&fresh, row, &columns, true, &theme());
        assert_eq!(line.spans[1].content, "+ ");
        assert_eq!(line.spans[1].style, theme().ok_style());
        assert!(line_text(&line).contains("will be enabled"));
    }

    #[test]
    fn a_row_nobody_enabled_carries_no_mark_and_says_nothing_about_ports() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, &columns(118, false), false, &theme());
        assert_eq!(
            line.spans[0].content, "   ",
            "no cursor on an unselected row"
        );
        assert_eq!(line.spans[1].content, "  ");
        let text = line_text(&line);
        assert!(!text.contains("internal"), "{text}");
        assert!(!text.contains(':'), "{text}");
    }

    #[test]
    fn the_status_column_carries_the_assessment_colour() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, &columns(118, false), true, &theme());
        let dot = line
            .spans
            .iter()
            .find(|s| s.content.starts_with('●'))
            .expect("the status column is there");
        assert_eq!(
            dot.style,
            theme().status_style(app.model(row).assessed_status)
        );
        assert_eq!(dot.style.fg, Some(theme().warn));
        assert!(line_text(&line).contains("● limited data"));
    }

    /// The strip under the table is what the details pane used to be, boiled
    /// down to the one line that says whether this model is worth opening.
    #[test]
    fn the_summary_strip_describes_the_row_under_the_cursor() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let lines = summary_lines(&app, 118, &theme());
        assert_eq!(lines.len(), 1, "one line, not three");

        let head = line_text(&lines[0]);
        assert!(head.starts_with("CHAP-EWARS"), "{head}");
        assert!(head.contains("● limited data"), "{head}");
        assert!(head.contains("1.0.2 (sha-"), "{head}");
        assert!(head.contains("not enabled"), "{head}");
        assert!(head.contains("requires population"), "{head}");
        assert!(head.ends_with("i for details"), "{head}");
        assert_eq!(head.chars().count(), 118, "the hint is right-aligned");

        // What the strip dropped is in the overlay, not on the line.
        for gone in ["Bayesian hierarchical", "defaults", "horizon", "user "] {
            assert!(!head.contains(gone), "{gone} is still on the strip: {head}");
        }

        // Too narrow for the covariates: they are cut, the hint stays.
        let narrow = line_text(&summary_lines(&app, 70, &theme())[0]);
        assert!(narrow.starts_with("CHAP-EWARS"), "{narrow}");
        assert!(
            narrow.ends_with("i for details"),
            "the hint always fits: {narrow}"
        );
        assert!(narrow.contains('~'), "the cut is marked: {narrow}");
        assert_eq!(narrow.chars().count(), 70);
    }

    /// With something unsaved, the strip stops describing a model and starts
    /// listing what saving would write.
    #[test]
    fn pending_changes_replace_the_summary_strip() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::Toggle);
        app.reduce(Action::Down);
        app.reduce(Action::Toggle);

        let screen = render(&app, 120, 40);
        assert!(screen.contains("2 pending changes"), "{screen}");
        assert!(
            screen.contains("nothing is written until you save"),
            "{screen}"
        );
        let removed = line_with(&screen, "the data volume");
        assert!(removed.contains("- CHAP-EWARS"), "{removed}");
        assert!(removed.contains("the data volume is kept"), "{removed}");
        let added = line_with(&screen, "enable at");
        assert!(added.contains("+ "), "{added}");
        assert!(added.contains("via chap-core"), "{added}");
        assert!(!screen.contains("i for details"), "{screen}");
        // And the header counts them, in the colour that says "unsaved".
        assert!(screen.contains("2 pending"), "{screen}");
    }

    /// A port change is not an enable, and the strip says which it is.
    #[test]
    fn a_changed_port_is_listed_as_an_update() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::PortPrompt);
        app.port_input.clear();
        for c in "5010".chars() {
            app.reduce(Action::PortChar(c));
        }
        app.reduce(Action::PortApply);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("1 pending change "), "{screen}");
        let line = line_with(&screen, "publish port 5010");
        assert!(line.contains("+ CHAP-EWARS"), "{line}");
        assert!(screen.contains("5010"), "the column shows it too");
    }

    /// The port dialog: what is being typed, what the row has now, and - when
    /// there is one - the reason the last entry was refused, all inside the
    /// box, with only the way out on the bar underneath.
    #[test]
    fn the_port_dialog_draws_the_state_and_what_it_refused() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::PortPrompt);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("Host port for CHAP-EWARS"), "{screen}");
        assert!(screen.contains("port  › auto_"), "{screen}");
        assert!(
            screen.contains("now: via chap-core · range 5001-5999 · api port 8000 is taken"),
            "{screen}"
        );
        assert!(
            screen
                .contains("[enter] apply  [esc] cancel  [auto] any free port  [none] no host port"),
            "the dialog carries its own keys:\n{screen}"
        );
        let bar = screen.lines().last().expect("a key bar");
        assert!(bar.trim() == "[esc] cancel", "{bar}");
        // Rounded like the other overlays, and centred.
        assert!(screen.contains('╭') && screen.contains('╯'), "{screen}");

        // A refusal keeps the box up with the reason inside it.
        app.port_input.clear();
        for c in "80".chars() {
            app.reduce(Action::PortChar(c));
        }
        app.reduce(Action::PortApply);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("port  › 80_"), "{screen}");
        assert!(
            screen.contains("port 80 is outside this project's range 5001-5999"),
            "{screen}"
        );
        assert!(screen.contains("Host port for CHAP-EWARS"), "still open");

        // Cancelling leaves nothing behind.
        app.reduce(Action::FilterCancel);
        let screen = render(&app, 120, 40);
        assert!(!screen.contains("Host port for"), "{screen}");
        assert!(!screen.contains("outside this project's range"), "{screen}");
    }

    /// The channel dialog: the two channels, what each resolves to, and which
    /// one the row follows today.
    #[test]
    fn the_channel_dialog_lists_both_channels_and_marks_the_one_in_force() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::ChannelPrompt);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("Channel for CHAP-EWARS"), "{screen}");
        let stable = line_with(&screen, "stable");
        assert!(stable.contains("▸ ✓ stable"), "{stable}");
        assert!(stable.contains("1.0.2"), "the version it resolves to");
        let latest = line_with(&screen, "latest");
        assert!(latest.contains("latest"), "{latest}");
        assert!(!latest.contains('▸'), "{latest}");
        assert!(
            screen.contains("[enter] apply  [esc] cancel  [j/k] move  [s/l] pick one"),
            "{screen}"
        );
        assert_eq!(
            screen.lines().last().expect("a key bar").trim(),
            "[esc] cancel"
        );

        // `l` moves the cursor, Enter takes it, and the list says so.
        app.reduce(Action::ChannelChar('l'));
        let screen = render(&app, 120, 40);
        assert!(line_with(&screen, "latest").contains("▸"), "{screen}");
        app.reduce(Action::ChannelApply);
        let screen = render(&app, 120, 40);
        assert!(!screen.contains("Channel for"), "{screen}");
        assert!(
            screen.contains("follow latest"),
            "the strip says what moved"
        );
    }

    #[test]
    fn the_details_overlay_opens_closes_and_scrolls() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let mut app = App::new(&registry, &state);
        assert!(!render(&app, 120, 40).contains("data dir"));

        app.reduce(Action::Info);
        let screen = render(&app, 120, 40);
        // Everything the old pane held is here.
        for needle in [
            // The colour word is only worth printing next to what it means.
            "● orange, shows promise on limited data",
            "http://localhost:5001",
            "1.0.2 (sha-8d4a7ea) · verified · channels stable, latest",
            "ghcr.io/chap-models/chapkit_ewars_model:sha-",
            "amd64 only",
            "/app/data",
            "chapkit:chapkit",
            "Bayesian hierarchical",
            "covariates",
            "period      monthly, weekly · horizon 0 to 100 periods",
            "target      disease cases",
            "maintainers",
            "author      ",
            "citation    ",
            "github.com/chap-models/chapkit_ewars_model",
            // The title carries the state, in the words the strip uses.
            "enabled on port 5001",
        ] {
            assert!(screen.contains(needle), "{needle} is missing:\n{screen}");
        }
        assert!(screen.contains("[esc] close"), "{screen}");
        assert!(screen.contains("[o] open repository"), "{screen}");
        assert!(screen.contains("[c] image ref"), "{screen}");

        // A short terminal cannot hold it all, so j and k move it.
        let short = render(&app, 80, 24);
        assert!(short.contains("Bayesian hierarchical"), "{short}");
        assert!(app.info_max.get() > 0, "the overlay has more to show");
        app.reduce(Action::Down);
        app.reduce(Action::Down);
        assert_eq!(app.info_scroll, 2);
        let scrolled = render(&app, 80, 24);
        assert_ne!(scrolled, short, "j scrolled the overlay");
        assert!(
            !scrolled.contains("Bayesian hierarchical"),
            "the top scrolled away:\n{scrolled}"
        );

        // It never scrolls past the end, and it comes back.
        for _ in 0..50 {
            app.reduce(Action::Down);
        }
        render(&app, 80, 24);
        assert_eq!(app.info_scroll, app.info_max.get());
        app.reduce(Action::Top);
        assert_eq!(app.info_scroll, 0);

        app.reduce(Action::Info);
        assert_eq!(app.mode, Mode::Browse);
        assert!(!render(&app, 120, 40).contains("data dir"));
    }

    /// The title is the one line that can collide: three things competing
    /// for the top border. The state is the one that has to survive.
    #[test]
    fn the_overlay_title_keeps_the_state_and_cuts_the_id() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::Info);

        let screen = render(&app, 120, 40);
        let wide = line_with(&screen, "CHAP-EWARS  chapkit_ewars_model");
        assert!(wide.contains("enabled, via chap-core"), "{wide}");
        assert!(
            !wide.contains("● limited data"),
            "the assessment has a row of its own: {wide}"
        );

        // The id gives way, the state does not, and they never touch.
        let title = line_text(&info_title(&app, app.selected().unwrap(), 50, &theme()));
        assert_eq!(title.chars().count(), 50);
        assert!(title.starts_with(" CHAP-EWARS"), "{title}");
        assert!(title.ends_with("enabled, via chap-core "), "{title}");
        assert!(title.contains('~'), "the id is cut: {title}");
        assert!(
            title.contains("  enabled"),
            "two columns of daylight, not a collision: {title}"
        );
    }

    /// The order is the one the CHAP Modeling App uses: what it does, what
    /// it is, who wrote it, then how this deployment runs it.
    #[test]
    fn the_overlay_leads_with_what_the_model_does() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let app = App::new(&registry, &state);
        let row = app.selected().expect("a row");
        let lines: Vec<String> = info_lines(&app, row, 83, &theme())
            .iter()
            .map(line_text)
            .collect();

        assert!(lines[0].starts_with("Bayesian hierarchical"), "{lines:?}");
        let at = |label: &str| {
            lines
                .iter()
                .position(|l| l.starts_with(label))
                .unwrap_or_else(|| panic!("no {label} row in {lines:?}"))
        };
        let order = [
            "version",
            "author",
            "status",
            "period",
            "target",
            "covariates",
            "reach",
            "image",
            "runtime",
            "data dir",
            "user",
            "maintainers",
            "repository",
            "citation",
        ];
        let mut last = 0;
        for label in order {
            let at = at(label);
            assert!(at > last, "{label} is out of order in {lines:?}");
            last = at;
        }
        // A blank line before the deployment facts, and before the credits.
        assert_eq!(lines[at("reach") - 1], "", "{lines:?}");
        assert_eq!(lines[at("maintainers") - 1], "", "{lines:?}");
        // Long values wrap under their label instead of being cut.
        assert!(!lines.iter().any(|l| l.contains('~')), "{lines:?}");
    }

    /// One version is the whole story on the `version` row; a model with a
    /// history lists the rest of it underneath.
    #[test]
    fn a_model_with_more_than_one_version_lists_them_all() {
        let mut registry = registry();
        let model = registry
            .models
            .iter_mut()
            .find(|m| m.id == EWARS)
            .expect("the ewars model");
        let mut older = model.versions[0].clone();
        older.version = "1.0.1".to_string();
        older.image_tag = "sha-0000001".to_string();
        older.status = VersionStatus::Deprecated;
        model.versions.push(older);

        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row");
        let lines: Vec<String> = info_lines(&app, row, 83, &theme())
            .iter()
            .map(line_text)
            .collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("version"))
            .expect("a version row");
        assert!(
            lines[at].contains("1.0.2 (sha-8d4a7ea) · verified · channels stable, latest"),
            "{:?}",
            lines[at]
        );
        assert!(
            lines[at + 1]
                .trim()
                .starts_with("1.0.1 (sha-0000001) · deprecated"),
            "the older release follows it: {:?}",
            lines[at + 1]
        );
        assert!(
            !lines[at + 1].contains("channels"),
            "no channel points at it: {:?}",
            lines[at + 1]
        );
    }

    #[test]
    fn the_palette_filters_and_runs_a_command() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Palette);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("Commands"), "{screen}");
        assert!(screen.contains("› _"), "{screen}");
        assert!(
            screen.contains(&format!("{0} of {0} commands", app.commands().len())),
            "{screen}"
        );
        assert!(
            screen.contains("Refresh registry"),
            "the strip lists them all"
        );

        for c in "po".chars() {
            app.reduce(Action::PaletteChar(c));
        }
        let screen = render(&app, 120, 40);
        assert!(screen.contains("› po_"), "{screen}");
        assert!(
            screen.contains("Set a host port for CHAP-EWARS"),
            "{screen}"
        );
        assert!(screen.contains("Remove the host port"), "{screen}");
        assert!(
            !screen.contains("Show or hide templates"),
            "the filter narrowed the list:\n{screen}"
        );
        assert!(
            screen.contains(&format!(
                "{} of {} commands",
                app.palette_matches().len(),
                app.commands().len()
            )),
            "{screen}"
        );

        // Typing something no command holds says so rather than showing an
        // empty box.
        app.reduce(Action::PaletteChar('z'));
        assert!(render(&app, 120, 40).contains("no command matches"));
        app.reduce(Action::PaletteBackspace);

        // Enter runs what the cursor is on, and the palette closes.
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Palette);
        for c in "enable or".chars() {
            app.reduce(Action::PaletteChar(c));
        }
        assert_eq!(app.palette_matches().len(), 1);
        app.reduce(Action::PaletteRun);
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.palette_query.is_empty());
        assert!(
            app.selected().expect("a row").enabled,
            "the row was toggled"
        );
        assert!(render(&app, 120, 40).contains("will be enabled"));
    }

    #[test]
    fn the_footer_follows_what_is_pending_and_what_is_filtered() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        // Wide enough for the whole bar, in the order the design gives.
        let wide = render(&app, 150, 40);
        let bar = wide.lines().last().expect("the bar is the last line");
        let order: Vec<&str> = [
            "[j/k] move",
            "[space] toggle",
            "[i] info",
            "[p] port",
            "[v] channel",
            "[t] templates",
            "[/] filter",
            "[s] save",
            "[ctrl+k] commands",
            "[?] help",
            "[q] quit",
        ]
        .into_iter()
        .collect();
        let mut at = 0;
        for entry in &order {
            let found = bar[at..]
                .find(entry)
                .unwrap_or_else(|| panic!("{entry} is missing or out of order:\n{bar}"));
            at += found + entry.len();
        }

        // At a hundred and twenty the display toggles give way first.
        let plain = render(&app, 120, 40);
        assert!(plain.contains("[j/k] move"), "{plain}");
        assert!(plain.contains("[p] port"), "{plain}");
        assert!(plain.contains("[ctrl+k] commands"), "{plain}");
        assert!(plain.contains("[s] save"), "{plain}");
        assert!(!plain.contains("[u] discard"), "{plain}");
        assert!(!plain.contains("[t] templates"), "{plain}");

        app.reduce(Action::Toggle);
        let pending = render(&app, 150, 40);
        assert!(pending.contains("[s] save 1 change "), "{pending}");
        assert!(pending.contains("[u] discard"), "{pending}");

        app.reduce(Action::StartFilter);
        for c in "arima".chars() {
            app.reduce(Action::FilterChar(c));
        }
        app.reduce(Action::FilterDone);
        let filtered = render(&app, 150, 40);
        assert!(filtered.contains("[esc] clear filter"), "{filtered}");
        assert!(!filtered.contains("[t] templates"), "{filtered}");
        assert!(!filtered.contains("[/] filter"), "{filtered}");
    }

    /// The save chip is filled rather than written, so it is the one thing on
    /// the bar that cannot be mistaken for another key.
    #[test]
    fn the_save_chip_is_filled_when_there_is_something_to_save() {
        let theme = theme();
        let hints = keys::keybar(Mode::Browse, 2, false);
        let spans = keybar(&hints, 200, &theme);
        let chip = spans
            .iter()
            .find(|span| span.content.contains("save 2 changes"))
            .expect("the chip is on the bar");
        assert_eq!(chip.style, theme.chip_style());
        assert_eq!(chip.style.bg, Some(theme.warn));

        // And the bar gives up the least useful keys before it overflows.
        let narrow = keybar(&hints, 44, &theme);
        let text: String = narrow.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.chars().count() <= 44, "{text}");
        assert!(text.contains("save 2 changes"), "{text}");
        assert!(text.contains("move"), "{text}");
        assert!(!text.contains("channel"), "{text}");
    }

    #[test]
    fn filter_mode_shows_what_is_being_typed_in_the_title() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::StartFilter);
        for c in "ewars".chars() {
            app.reduce(Action::FilterChar(c));
        }
        let screen = render(&app, 120, 40);
        assert!(
            screen.contains("chaps · models · filter ewars_"),
            "{screen}"
        );
        assert!(screen.contains("CHAP-EWARS"));
        assert!(screen.contains("1 of 7 match \"ewars\""), "{screen}");
        assert!(screen.contains("[esc] clear"), "{screen}");
    }

    #[test]
    fn templates_and_manual_entries_get_a_kind_column() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 120, 40);
        assert!(!screen.contains("KIND"), "{screen}");

        app.reduce(Action::ToggleTemplates);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("KIND"), "{screen}");
        assert!(screen.contains("template"), "{screen}");
        let row = line_with(&screen, "CHAP-EWARS  ");
        let head = line_with(&screen, "MODEL");
        assert_eq!(column_of(head, "KIND"), column_of(row, "model "));
    }

    /// A model the deployment added itself is marked in the table and named
    /// as such in the overlay, so the browser never presents a local
    /// definition as a reviewed marketplace entry.
    #[test]
    fn a_manually_added_model_is_marked_and_named() {
        let mut state = ProjectState::default();
        // An id the marketplace does not list: what a deployment can add.
        state.manual.insert(
            "example_manual_model".to_string(),
            crate::project::ManualModel {
                service_id: "example-manual-model".into(),
                display_name: "example_manual_model".into(),
                repository: Some("https://github.com/example/example_manual_model".into()),
                image: "ghcr.io/example/example_manual_model".into(),
                tag: "sha-b1d6c31".into(),
                commit: None,
                follow: Some("main".into()),
                data_dir: Some("/work/data".into()),
                user: Some("10001:10001".into()),
                runtime_amd64: true,
                added: "2026-09-24".into(),
            },
        );
        let mut registry = registry();
        assert!(registry.with_manual(&state.manual).is_empty());

        let mut app = App::new(&registry, &state);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("KIND"), "{screen}");
        let row = line_with(&screen, "example_manual_model");
        assert!(row.contains("manual "), "{row}");
        assert!(!row.contains("template"), "{row}");

        // The overlay for that row says what kind of entry it is.
        while app.selected().map(|row| app.model(row).id.as_str()) != Some("example_manual_model") {
            app.reduce(Action::Down);
        }
        // The row shows the tag as the tag, not as a version number.
        let screen = render(&app, 120, 40);
        assert!(screen.contains("sha-b1"), "{screen}");
        assert!(!screen.contains("vsha-"), "{screen}");

        app.reduce(Action::Info);
        let screen = render(&app, 120, 40);
        assert!(screen.contains("manual (added with `chaps"), "{screen}");
        assert!(
            screen.contains("ghcr.io/example/example_manual_model:sha-"),
            "{screen}"
        );
    }

    #[test]
    fn the_overlays_render_on_top_in_rounded_boxes() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Help);
        let screen = render(&app, 100, 30);
        assert!(screen.contains("Keys"));
        assert!(screen.contains("enable or disable the model"));
        assert!(screen.contains("the command palette"), "{screen}");
        assert!(screen.contains("discard the pending changes"), "{screen}");
        assert!(screen.contains('╭'));

        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Toggle);
        app.reduce(Action::Quit);
        let screen = render(&app, 100, 30);
        assert!(screen.contains("Quit"));
        assert!(screen.contains("unsaved changes"));
        assert!(screen.contains("Quit anyway?"));
    }

    #[test]
    fn an_empty_filter_result_still_draws() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::StartFilter);
        for c in "zzzz".chars() {
            app.reduce(Action::FilterChar(c));
        }
        let screen = render(&app, 120, 40);
        assert!(screen.contains("nothing matches this filter"));
        // And `i` has nothing to open.
        app.reduce(Action::FilterDone);
        app.reduce(Action::Info);
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn the_layout_holds_from_a_tiny_terminal_to_a_huge_one() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        for (width, height) in [(60, 16), (80, 24), (200, 60)] {
            let screen = render(&app, width, height);
            assert!(screen.contains("chaps · models"), "{width}x{height}");
            assert!(screen.contains("Marketplace"), "{width}x{height}");
            assert!(screen.contains("CHAP-EWARS"), "{width}x{height}");
            assert!(screen.contains("[space] toggle"), "{width}x{height}");
            assert_eq!(
                screen.lines().count(),
                height as usize,
                "the frame fills the terminal exactly"
            );
            for line in screen.lines() {
                assert_eq!(
                    line.chars().count(),
                    width as usize,
                    "a line overflowed at {width}x{height}:\n{screen}"
                );
            }
        }
    }

    /// Eighty columns is the terminal chaps has to assume: the id gives way
    /// first, marked, and the strip's lines are cut rather than wrapped.
    #[test]
    fn eighty_columns_truncates_the_id_and_the_strip() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 80, 24);
        let row = line_with(&screen, "Rwanda Malaria BYM");
        assert!(row.contains("chapkit_rwa~"), "{row}");
        assert!(!row.contains("chapkit_rwanda_malaria_bym_model"), "{row}");
        assert!(
            screen.contains("● not for use"),
            "the status column survives"
        );

        let strip = line_with(&screen, "i for details");
        assert!(strip.contains("CHAP-EWARS"), "{strip}");
        assert!(
            strip.contains('~'),
            "the strip is cut, not wrapped: {strip}"
        );
        assert!(
            strip
                .trim_end_matches('│')
                .trim_end()
                .ends_with("i for details"),
            "the hint survives eighty columns: {strip}"
        );
        assert_eq!(strip.chars().count(), 80);
        assert_eq!(
            screen.matches("i for details").count(),
            1,
            "the strip is one row, so the hint is on it once:\n{screen}"
        );
    }

    #[test]
    fn the_title_bar_drops_what_does_not_fit_instead_of_overflowing() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        // Wide: both halves.
        let wide = render(&app, 140, 20);
        let first = wide.lines().next().unwrap();
        assert!(first.contains("registry: embedded"));
        assert!(first.contains(&format!("{} models", registry.models.len())));

        // Narrow: the counts are worth more than where the file came from.
        let narrow = render(&app, 60, 16);
        let first = narrow.lines().next().unwrap();
        assert_eq!(first.chars().count(), 60);
        assert!(!first.contains("registry: embedded"), "{first}");
        assert!(first.contains("chaps · models"));
    }

    #[test]
    fn a_cached_catalogue_says_how_old_it_is() {
        assert_eq!(
            registry_label(&Provenance::Cache { age_secs: 120 }),
            "registry: cache · 2 min"
        );
        assert_eq!(
            registry_label(&Provenance::StaleCache { age_secs: 172_800 }),
            "registry: stale cache · 2 d"
        );
        assert_eq!(registry_label(&Provenance::Network), "registry: network");
        assert_eq!(registry_label(&Provenance::Embedded), "registry: embedded");
        assert_eq!(short_age(0), "0 sec");
        assert_eq!(short_age(59), "59 sec");
        assert_eq!(short_age(60), "1 min");
        assert_eq!(short_age(7200), "2 hr");
    }

    #[test]
    fn a_monochrome_theme_draws_the_same_shape_without_colour() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let app = App::new(&registry, &state);
        assert_eq!(
            render_with(&app, 100, 30, &Theme::monochrome()),
            render(&app, 100, 30),
            "NO_COLOR changes the styles, never the characters"
        );

        let mono = Theme::monochrome();
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, &columns(98, false), true, &mono);
        assert!(line.spans.iter().all(|s| s.style.fg.is_none()));
        assert_ne!(
            line.spans[1].style,
            ratatui::style::Style::default(),
            "the tick still stands out"
        );
    }

    #[test]
    fn narrow_tables_drop_columns_instead_of_wrapping() {
        // Wide enough for everything, with the id capped so a wide terminal
        // does not push the columns apart.
        let wide = columns(160, false);
        assert_eq!(wide.id, ID_MAX);
        assert_eq!(wide.status, STATUS_W);
        assert_eq!(wide.port, PORT_W);
        assert_eq!(wide.kind, 0);

        assert_eq!(columns(160, true).kind, KIND_W);

        // Eighty columns: everything, with a shorter id and the narrow form
        // of the enabled column.
        let eighty = columns(78, false);
        assert!(eighty.id >= ID_MIN, "{eighty:?}");
        assert!(eighty.id < ID_MAX);
        assert_eq!(eighty.status, STATUS_W, "the assessment keeps its words");
        assert_eq!(eighty.port, PORT_MIN);

        // Sixty: the id goes first, then the enabled column narrows.
        let sixty = columns(58, false);
        assert_eq!(sixty.id, 0, "{sixty:?}");
        assert_eq!(sixty.version, VERSION_W);

        // Tiny: the name and nothing else, and never a width below zero.
        let tiny = columns(20, false);
        assert_eq!(tiny.id, 0);
        assert_eq!(tiny.status, 0);
        assert_eq!(tiny.model, 15);
        assert_eq!(columns(0, true), columns(0, false));

        for inner in 0..200usize {
            let c = columns(inner, inner % 2 == 0);
            let used = MARKER_W
                + [c.model, c.kind, c.id, c.status, c.version, c.port]
                    .iter()
                    .map(|w| if *w > 0 { w + 1 } else { 0 })
                    .sum::<usize>();
            assert!(
                used <= inner.max(MARKER_W) + 1,
                "{inner}: {c:?} needs {used}"
            );
        }
    }

    #[test]
    fn small_terminals_do_not_panic() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        for (width, height) in [
            (1, 1),
            (2, 2),
            (8, 3),
            (20, 5),
            (40, 10),
            (60, 16),
            (79, 23),
            (80, 24),
            (200, 60),
        ] {
            render(&app, width, height);
            render_with(&app, width, height, &Theme::monochrome());
        }
        // Overlays are the easiest thing to draw outside a tiny screen.
        for action in [
            Action::Help,
            Action::Info,
            Action::Palette,
            Action::PortPrompt,
            Action::ChannelPrompt,
        ] {
            // Enabled, so the port dialog has something to open on.
            let mut app = App::new(&registry, &state_with_ewars(&registry, Some(5001)));
            app.reduce(action.clone());
            assert_ne!(app.mode, Mode::Browse, "{action:?} opened nothing");
            for (width, height) in [(1, 1), (10, 4), (30, 6), (60, 16), (80, 24)] {
                render(&app, width, height);
            }
        }
        app.reduce(Action::Toggle);
        app.reduce(Action::Quit);
        for (width, height) in [(1, 1), (10, 4), (30, 6), (60, 16), (80, 24)] {
            render(&app, width, height);
        }
    }

    #[test]
    fn the_box_gives_up_its_rules_before_its_rows() {
        // Roomy and idle: the one-row strip leaves the rest to the list.
        assert_eq!(split_body(20, 1), [1, 1, 16, 1, 1]);
        // Roomy, with a three-line pending strip.
        assert_eq!(split_body(20, 3), [1, 1, 14, 1, 3]);
        // Tight: the rules go, then the strip, then the headings.
        assert_eq!(split_body(7, 3), [1, 1, 1, 1, 3]);
        assert_eq!(split_body(5, 3), [1, 0, 1, 0, 3]);
        assert_eq!(split_body(2, 3), [0, 0, 1, 0, 1]);
        assert_eq!(split_body(1, 3), [0, 0, 1, 0, 0]);
        assert_eq!(split_body(0, 3), [0, 0, 0, 0, 0]);
        for h in 0..40u16 {
            for want in 0..8u16 {
                assert_eq!(split_body(h, want).iter().sum::<u16>(), h, "{h}/{want}");
            }
        }
    }

    #[test]
    fn zz_dump_docs_mock() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let app = App::new(&registry, &state);
        eprintln!("=== DOCS BROWSER 120x11 ===");
        eprintln!("{}", render(&app, 120, 11));
        let mut app = App::new(&registry, &ProjectState::default());
        while app.selected().map(|r| app.model(r).display_name.clone())
            != Some("Rwanda Malaria BYM".to_string())
        {
            app.reduce(Action::Down);
        }
        app.reduce(Action::Info);
        eprintln!("=== DOCS OVERLAY 120x40 ===");
        eprintln!("{}", render(&app, 120, 40));
    }

    #[test]
    fn zz_dump_buffers() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        eprintln!("=== IDLE 120 (default cursor) ===");
        eprintln!("{}", render(&app, 120, 40));
        while app.selected().map(|r| app.model(r).display_name.clone())
            != Some("Rwanda Malaria BYM".to_string())
        {
            app.reduce(Action::Down);
        }
        eprintln!("=== IDLE 120 (Rwanda) ===");
        eprintln!("{}", render(&app, 120, 40));
        eprintln!("=== OVERLAY 120x40 (Rwanda) ===");
        app.reduce(Action::Info);
        eprintln!("{}", render(&app, 120, 40));
        app.reduce(Action::Info);
        eprintln!("=== IDLE 80x24 ===");
        let plain = App::new(&registry, &ProjectState::default());
        eprintln!("{}", render(&plain, 80, 24));
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Toggle);
        app.reduce(Action::Down);
        app.reduce(Action::Toggle);
        eprintln!("=== PENDING 120 ===");
        eprintln!("{}", render(&app, 120, 40));

        let app = App::new(&registry, &state_with_ewars(&registry, None));
        let mut terminal = Terminal::new(TestBackend::new(120, 11)).expect("test backend starts");
        terminal
            .draw(|frame| draw(frame, &app, &theme()))
            .expect("draw");
        eprintln!("=== SVG ===");
        eprintln!(
            "{}",
            crate::tui::screenshot::svg(terminal.backend().buffer(), &theme())
        );
    }

    #[test]
    fn fit_pads_and_truncates() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcd", 4), "abcd");
        assert_eq!(fit("abcdef", 4), "abc~");
        assert_eq!(fit("abc", 0), "");
        assert_eq!(fit("", 3), "   ");
        // Multi-byte characters must not be cut in half.
        assert_eq!(fit("æøå-modell", 6).chars().count(), 6);
        assert_eq!(fit_soft("ab", 4), "ab");
        assert_eq!(fit_soft("abcdef", 4), "abc~");
    }

    #[test]
    fn truncating_a_line_marks_where_it_was_cut() {
        let theme = theme();
        let spans = || {
            vec![
                Span::raw("requires population"),
                Span::styled("   defaults ", theme.dim_style()),
                Span::raw("rainfall"),
            ]
        };
        let mut line = spans();
        truncate(&mut line, 100);
        assert_eq!(width_of(&line), 39, "nothing to cut, nothing cut");

        let mut line = spans();
        truncate(&mut line, 24);
        let text: String = line.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), 24);
        assert!(text.ends_with('~'), "{text}");

        // A cut that lands exactly on a span boundary still marks itself.
        let mut line = spans();
        truncate(&mut line, 19);
        let text: String = line.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "requires populatio~");

        let mut line = spans();
        truncate(&mut line, 0);
        assert!(line.is_empty());
    }

    #[test]
    fn wrapping_breaks_on_spaces_and_then_on_anything() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("", 10), Vec::<String>::new());
        assert_eq!(wrap("word", 0), Vec::<String>::new());
        // A word longer than the line is broken rather than dropped.
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        for line in wrap("a very long sentence about models and ports", 11) {
            assert!(line.chars().count() <= 11, "{line}");
        }
    }

    #[test]
    fn centred_popups_stay_inside_the_screen() {
        let area = Rect::new(0, 0, 10, 4);
        let popup = centered(area, 52, 12);
        assert_eq!(popup, area);

        let area = Rect::new(0, 0, 100, 30);
        let popup = centered(area, 52, 12);
        assert_eq!(popup.width, 52);
        assert_eq!(popup.height, 12);
        assert!(popup.x + popup.width <= area.width);
        assert!(popup.y + popup.height <= area.height);
    }
}
