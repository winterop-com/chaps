//! The two tables: the models page's and the components page's, their
//! column widths, headings and rows.

use super::text::fit;
use crate::tui::app::{App, ComponentLine, PortWant, Row};
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

/// A space, the cursor column, and the enabled mark, each with a space.
pub(super) const MARKER_W: usize = 5;
pub(super) const MODEL_W: usize = 22;
/// `template` is the longest thing the kind column says.
pub(super) const KIND_W: usize = 8;
/// `● experimental`: the dot, and the longest thing the scale means.
pub(super) const STATUS_W: usize = 14;
pub(super) const VERSION_W: usize = 8;
/// `will be disabled`, the longest thing the port column says.
pub(super) const PORT_W: usize = 16;
/// Enough for `via chap-core`, when the full column does not fit.
pub(super) const PORT_MIN: usize = 13;
pub(super) const ID_MIN: usize = 10;
/// The components page's columns: the name, whether this deployment has it,
/// and where it is reached. What it is takes the rest of the line.
pub(super) const COMPONENT_W: usize = 12;
pub(super) const STATE_W: usize = 8;
pub(super) const REACH_W: usize = 24;
/// Enough for `internal`, or the start of a URL, when the full column does
/// not fit: where a component is reached is worth more than the sentence
/// saying what it is.
pub(super) const REACH_MIN: usize = 10;
/// The id column stops growing here: a table that stretches an id across a
/// wide terminal only puts distance between the columns that matter.
pub(super) const ID_MAX: usize = 34;

/// The widths of the table's columns, so the headings and the rows agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Columns {
    pub(super) model: usize,
    pub(super) kind: usize,
    pub(super) id: usize,
    pub(super) status: usize,
    pub(super) version: usize,
    pub(super) port: usize,
}

/// Hand out the width the table has, widest column last.
///
/// Every column but the name is optional: they switch on in the order they
/// stop costing the id its readable width, so a narrow terminal loses whole
/// columns instead of wrapping a row.
pub(super) fn columns(inner: usize, kind: bool) -> Columns {
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
pub(super) fn kind_column(app: &App) -> bool {
    app.show_templates || app.rows.iter().any(|row| app.model(row).manual)
}

/// `MODEL  ID  STATUS  VERSION  ENABLED`, over the columns they label.
pub(super) fn draw_head(frame: &mut Frame, area: Rect, columns: &Columns, theme: &Theme) {
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
pub(super) fn draw_rule(frame: &mut Frame, area: Rect, theme: &Theme) {
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

pub(super) fn draw_list(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    columns: &Columns,
    theme: &Theme,
) {
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
pub(super) fn row_line<'a>(
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
pub(super) struct ComponentColumns {
    pub(super) component: usize,
    pub(super) state: usize,
    pub(super) reach: usize,
    pub(super) what: usize,
}

/// Hand out the width the components table has.
///
/// The name, the state and where it is reached come first, and the sentence
/// saying what the component is takes whatever is left: a narrow terminal
/// loses the end of that rather than losing which row is which or where it
/// answers.
pub(super) fn component_columns(inner: usize) -> ComponentColumns {
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
pub(super) fn draw_component_head(
    frame: &mut Frame,
    area: Rect,
    columns: &ComponentColumns,
    theme: &Theme,
) {
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

pub(super) fn draw_component_list(
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
