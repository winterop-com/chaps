//! TUI rendering: header, model list, details pane, key bar.
//!
//! Owned by agent C.
//!
//! Drawing never mutates the [`App`]: everything on screen is derived from the
//! state the reducer produced. The layout degrades on narrow terminals by
//! dropping columns rather than wrapping or panicking.

use crate::registry::{AssessedStatus, Channel, Version, VersionStatus};
use crate::tui::app::{App, Mode, Row};
use crate::tui::keys;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// Draw one frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header_area, app);

    let [list_area, details_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(body_area);
    draw_list(frame, list_area, app);
    draw_details(frame, details_area, app);

    draw_footer(frame, footer_area, app);

    match app.mode {
        Mode::Help => draw_help(frame, area),
        Mode::ConfirmQuit => draw_confirm(frame, area),
        _ => {}
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let counts = app.counts();
    let title = Line::from(vec![
        Span::styled(
            "chaps models",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            format!("registry: {}", app.registry.provenance.label()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let summary = Line::from(vec![
        Span::raw(format!(
            "{} models, {} enabled, ",
            counts.total, counts.enabled
        )),
        Span::styled(
            format!(
                "{} pending change{}",
                counts.pending,
                plural(counts.pending)
            ),
            if counts.pending > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
    ]);
    frame.render_widget(Paragraph::new(vec![title, summary]), area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" models ");
    let inner_width = area.width.saturating_sub(2);

    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|row_idx| ListItem::new(row_line(app, &app.rows[*row_idx], inner_width)))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.visible.is_empty() {
        state.select(Some(app.cursor.min(app.visible.len() - 1)));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// One list line: `[x] Display Name   id   status   vX.Y.Z   :port`.
///
/// Columns drop off from the right as the pane narrows, so the checkbox and
/// the name are always readable.
fn row_line<'a>(app: &App, row: &Row, width: u16) -> Line<'a> {
    let model = app.model(row);
    // Two columns go to the highlight symbol. The remaining columns switch on
    // in the order they become affordable, so nothing important is cut off.
    let width = width.saturating_sub(2);
    let show_version = width >= 41;
    let show_status = width >= 49;
    let show_id = width >= 80;

    let mut spans = vec![
        Span::styled(
            if row.enabled { "[x] " } else { "[ ] " },
            if row.enabled {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::raw(fit(&model.display_name, 22)),
    ];
    // The marker comes before the optional columns: a template must be
    // recognisable even in a pane too narrow for anything else.
    if model.is_template() {
        spans.push(Span::styled(
            " [template]",
            Style::default().fg(Color::Magenta),
        ));
    }
    if show_id {
        spans.push(Span::styled(
            format!(" {}", fit(&model.id, 30)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if show_status {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit(status_label(model.assessed_status), 7),
            status_style(model.assessed_status),
        ));
    }
    if show_version {
        let version = app
            .resolved(row)
            .map(|v| format!("v{}", v.version))
            .unwrap_or_else(|| "-".to_string());
        spans.push(Span::raw(format!(" {}", fit(&version, 8))));
    }
    if let Some(port) = row.port {
        spans.push(Span::styled(
            format!(" :{port}"),
            Style::default().fg(Color::Blue),
        ));
    }
    Line::from(spans)
}

fn draw_details(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" details ");
    let Some(row) = app.selected() else {
        frame.render_widget(
            Paragraph::new("nothing matches this filter").block(block),
            area,
        );
        return;
    };

    let model = app.model(row);
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        model.display_name.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        model.id.clone(),
        Style::default().fg(Color::DarkGray),
    )));

    // What this project already runs outranks the catalogue blurb: a short
    // pane only shows the top of the paragraph.
    if let Some(recorded) = app.recorded(row) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "enabled in this project",
            Style::default().fg(Color::Green),
        )));
        lines.push(field("port", &recorded.host_port.to_string()));
        lines.push(field(
            "pinned",
            &format!("{} ({})", recorded.version, recorded.image_tag),
        ));
        lines.push(field("data dir", &recorded.data_dir));
        lines.push(field("user", &recorded.user));
    }

    lines.push(Line::raw(""));
    lines.push(Line::raw(model.summary.clone()));
    lines.push(Line::raw(""));

    lines.push(field(
        "kind",
        if model.is_template() {
            "template (scaffolding, not a forecasting model)"
        } else {
            "model"
        },
    ));
    lines.push(Line::from(vec![
        Span::styled("status      ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            status_label(model.assessed_status).to_string(),
            status_style(model.assessed_status),
        ),
    ]));
    lines.push(field("channel", channel_label(row.channel)));

    match app.resolved(row) {
        Some(version) => {
            lines.push(field(
                "image",
                &format!("{}:{}", model.source.image, version.image_tag),
            ));
            lines.push(field(
                "version",
                &format!("{} ({})", version.version, version_status(version)),
            ));
        }
        None => lines.push(field("image", "unresolved: this channel has no version")),
    }

    let runtime = if model.needs_amd64() {
        format!("{} (amd64 only)", model.source.runtime_image)
    } else {
        model.source.runtime_image.clone()
    };
    lines.push(field("runtime", &runtime));
    lines.push(field(
        "periods",
        &model.compatibility.period_types.join(", "),
    ));
    lines.push(field(
        "horizon",
        &format!(
            "{} to {} periods",
            model.compatibility.min_prediction_periods, model.compatibility.max_prediction_periods
        ),
    ));
    lines.push(field(
        "geo",
        if model.compatibility.requires_geo {
            "polygons required"
        } else {
            "not required"
        },
    ));
    lines.push(field(
        "covariates",
        &list_or_dash(&model.covariates.required),
    ));
    lines.push(field("defaults", &list_or_dash(&model.covariates.defaults)));

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "versions",
        Style::default().fg(Color::DarkGray),
    )));
    for version in &model.versions {
        let mut marks: Vec<&str> = Vec::new();
        if version.version == model.channels.stable {
            marks.push("stable");
        }
        if version.version == model.channels.latest {
            marks.push("latest");
        }
        let marker = if marks.is_empty() {
            String::new()
        } else {
            format!("  <- {}", marks.join(", "))
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {}  ", fit(&version.version, 10))),
            Span::styled(
                fit(version_status(version), 10),
                version_style(version.status),
            ),
            Span::styled(marker, Style::default().fg(Color::Cyan)),
        ]));
    }

    if !model.maintainers.is_empty() {
        lines.push(Line::raw(""));
        lines.push(field("maintainers", &model.maintainers.join(", ")));
    }
    lines.push(field("author", &model.attribution.author));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let line = if let Some(message) = &app.message {
        Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(Color::Yellow),
        ))
    } else if app.mode == Mode::Filter {
        Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                app.filter.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("_   ", Style::default().fg(Color::Cyan)),
            Span::styled(
                keys::keybar(Mode::Filter).to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        let mut spans = Vec::new();
        if !app.filter.is_empty() {
            spans.push(Span::styled(
                format!(" filter: {}  ", app.filter),
                Style::default().fg(Color::Cyan),
            ));
        } else {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            keys::keybar(app.mode).to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let entries = keys::help_entries();
    let width = 52u16;
    let height = entries.len() as u16 + 2;
    let popup = centered(area, width, height);

    let lines: Vec<Line> = entries
        .iter()
        .map(|&(key, what)| {
            Line::from(vec![
                Span::styled(
                    format!(" {}", fit(key, 20)),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(what.to_string()),
            ])
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys ")),
        popup,
    );
}

fn draw_confirm(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 46, 4);
    let lines = vec![
        Line::raw(" There are unsaved changes."),
        Line::from(vec![
            Span::raw(" Quit anyway? "),
            Span::styled("y", Style::default().fg(Color::Red)),
            Span::raw(" / "),
            Span::styled("n", Style::default().fg(Color::Green)),
        ]),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" quit ")),
        popup,
    );
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

fn field<'a>(label: &str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(fit(label, 12), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

fn list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
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

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn channel_label(channel: Channel) -> &'static str {
    channel.as_str()
}

fn status_label(status: AssessedStatus) -> &'static str {
    match status {
        AssessedStatus::Green => "green",
        AssessedStatus::Yellow => "yellow",
        AssessedStatus::Orange => "orange",
        AssessedStatus::Red => "red",
        AssessedStatus::Gray => "gray",
    }
}

fn status_style(status: AssessedStatus) -> Style {
    match status {
        AssessedStatus::Green => Style::default().fg(Color::Green),
        AssessedStatus::Yellow => Style::default().fg(Color::Yellow),
        // No orange in the 16-colour palette; light red is the closest thing
        // every terminal renders.
        AssessedStatus::Orange => Style::default().fg(Color::LightRed),
        AssessedStatus::Red => Style::default().fg(Color::Red),
        AssessedStatus::Gray => Style::default().fg(Color::DarkGray),
    }
}

fn version_status(version: &Version) -> &'static str {
    match version.status {
        VersionStatus::Verified => "verified",
        VersionStatus::Unstable => "unstable",
        VersionStatus::Deprecated => "deprecated",
        VersionStatus::Yanked => "yanked",
    }
}

fn version_style(status: VersionStatus) -> Style {
    match status {
        VersionStatus::Verified => Style::default().fg(Color::Green),
        VersionStatus::Unstable => Style::default().fg(Color::Yellow),
        VersionStatus::Deprecated => Style::default().fg(Color::LightRed),
        VersionStatus::Yanked => Style::default().fg(Color::Red).crossed_out(),
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

    fn registry() -> Registry {
        load_embedded().expect("embedded snapshot parses")
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("test backend starts");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
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

    #[test]
    fn a_normal_terminal_shows_the_header_list_and_details() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 140, 40);
        assert!(screen.contains("chaps models"));
        assert!(screen.contains("registry: embedded"));
        assert!(screen.contains("6 models, 0 enabled, 0 pending changes"));
        assert!(screen.contains("CHAP-EWARS"));
        assert!(screen.contains("chapkit_ewars_model"));
        assert!(screen.contains("space toggle"));
        // The details pane describes the row under the cursor.
        assert!(screen.contains("verified"));
        assert!(screen.contains("amd64"));
    }

    #[test]
    fn enabled_rows_are_checked_and_show_their_port() {
        let registry = registry();
        let mut state = ProjectState::default();
        let model = registry.get("chapkit_ewars_model").unwrap();
        state.models.insert(
            model.id.clone(),
            crate::project::EnabledModel {
                service_id: model.service_id.clone(),
                image: model.source.image.clone(),
                image_tag: "sha-fa880a1".into(),
                version: model.channels.stable.clone(),
                channel: Some(Channel::Stable),
                host_port: 5001,
                data_dir: "/app/data".into(),
                user: "chapkit:chapkit".into(),
                platform: Some("linux/amd64".into()),
                compose_file: "compose.chapkit-ewars-model.yml".into(),
            },
        );
        let app = App::new(&registry, &state);
        let screen = render(&app, 100, 30);
        assert!(screen.contains("[x]"));
        assert!(screen.contains(":5001"));
        assert!(screen.contains("enabled in this project"));
    }

    #[test]
    fn templates_are_marked_when_shown() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        assert!(!render(&app, 100, 30).contains("[template]"));
        app.reduce(Action::ToggleTemplates);
        assert!(render(&app, 100, 30).contains("[template]"));
    }

    #[test]
    fn the_overlays_render_on_top() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Help);
        let screen = render(&app, 100, 30);
        assert!(screen.contains("keys"));
        assert!(screen.contains("enable or disable the model"));

        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::Toggle);
        app.reduce(Action::Quit);
        let screen = render(&app, 100, 30);
        assert!(screen.contains("unsaved changes"));
        assert!(screen.contains("Quit anyway?"));
    }

    #[test]
    fn filter_mode_shows_what_is_being_typed() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::StartFilter);
        for c in "ewars".chars() {
            app.reduce(Action::FilterChar(c));
        }
        let screen = render(&app, 100, 30);
        assert!(screen.contains("filter: ewars"));
        assert!(screen.contains("CHAP-EWARS"));
    }

    #[test]
    fn an_empty_filter_result_still_draws() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::StartFilter);
        for c in "zzzz".chars() {
            app.reduce(Action::FilterChar(c));
        }
        let screen = render(&app, 100, 30);
        assert!(screen.contains("nothing matches this filter"));
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
            (79, 23),
            (80, 24),
            (200, 60),
        ] {
            render(&app, width, height);
        }
        // Overlays are the easiest thing to draw outside a tiny screen.
        app.reduce(Action::Help);
        for (width, height) in [(1, 1), (10, 4), (30, 6), (80, 24)] {
            render(&app, width, height);
        }
        app.reduce(Action::Help);
        app.reduce(Action::Toggle);
        app.reduce(Action::Quit);
        for (width, height) in [(1, 1), (10, 4), (30, 6), (80, 24)] {
            render(&app, width, height);
        }
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn narrow_rows_drop_columns_instead_of_wrapping() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");

        let wide = line_text(&row_line(&app, row, 120));
        assert!(wide.contains("CHAP-EWARS"));
        assert!(
            wide.contains("chapkit_ewars_model"),
            "the id fits when wide"
        );
        assert!(wide.contains("orange"), "the assessed status has a column");

        let narrow = line_text(&row_line(&app, row, 30));
        assert!(narrow.contains("CHAP-EWARS"), "the name always fits");
        assert!(
            !narrow.contains("chapkit_ewars_model"),
            "the id column drops out of a narrow pane"
        );
        assert!(!narrow.contains("orange"), "so does the status column");
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
