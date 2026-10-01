//! The overlays that take a key or a line of input: the port, channel and
//! DHIS2 version dialogs, the command palette, the key help and the quit
//! confirmation.

use super::text::{centered, channel_label, fit, overlay, width_of_line, with_tail, wrap};
use super::{bar_width, keybar};
use crate::registry::Channel;
use crate::tui::app::{App, Command, Page, PortWant};
use crate::tui::keys::{self, Hint};
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// The host port dialog: what is being typed, what the row has now, why the
/// last thing typed was refused, and the keys that end it.
pub(super) fn draw_port_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
pub(super) fn draw_channel_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
pub(super) fn draw_dhis2_version_dialog(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
pub(super) fn draw_dialog(
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

/// The command palette: a filter line, what it matched, and every command it
/// could have matched.
pub(super) fn draw_palette(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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

pub(super) fn draw_help(frame: &mut Frame, area: Rect, theme: &Theme) {
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

pub(super) fn draw_confirm(frame: &mut Frame, area: Rect, theme: &Theme) {
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
