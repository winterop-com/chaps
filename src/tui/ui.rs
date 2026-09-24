//! TUI rendering: title bar, marketplace list, details pane, key bar.
//!
//! Owned by agent C.
//!
//! Drawing never mutates the [`App`]: everything on screen is derived from the
//! state the reducer produced, and every colour comes from the [`Theme`]. The
//! layout degrades on narrow terminals by dropping columns rather than
//! wrapping or panicking.

use crate::registry::{Channel, Version, VersionStatus};
use crate::tui::app::{App, Mode, Row};
use crate::tui::keys;
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// The label column in the details pane.
const LABEL_WIDTH: usize = 12;

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

    let [list_area, details_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(body_area);
    draw_list(frame, list_area, app, theme);
    draw_details(frame, details_area, app, theme);

    draw_footer(frame, footer_area, app, theme);

    match app.mode {
        Mode::Help => draw_help(frame, area, theme),
        Mode::ConfirmQuit => draw_confirm(frame, area, theme),
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
    let left = vec![
        Span::styled("chaps", theme.accent_style()),
        Span::styled(" · ", theme.dim_style()),
        Span::styled("models", theme.label_style()),
    ];

    let provenance = Span::styled(
        format!("registry: {}", app.registry.provenance.label()),
        theme.dim_style(),
    );
    let totals = Span::styled(
        format!("{} models, {} enabled, ", counts.total, counts.enabled),
        theme.dim_style(),
    );
    let pending = Span::styled(
        format!(
            "{} pending change{}",
            counts.pending,
            plural(counts.pending)
        ),
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

fn draw_list(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // The list is the only thing the keys act on, so it always holds focus.
    let block = pane(theme, "Marketplace", true);
    let inner_width = area.width.saturating_sub(2);

    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|row_idx| ListItem::new(row_line(app, &app.rows[*row_idx], inner_width, theme)))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_symbol("▸ ")
        .highlight_style(theme.selection_style());

    let mut state = ListState::default();
    if !app.visible.is_empty() {
        state.select(Some(app.cursor.min(app.visible.len() - 1)));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// One list line: `✓ Display Name   id   status   vX.Y.Z   :port`.
///
/// Columns drop off from the right as the pane narrows, so the mark and the
/// name are always readable.
fn row_line<'a>(app: &App, row: &Row, width: u16, theme: &Theme) -> Line<'a> {
    let model = app.model(row);
    // Two columns go to the highlight symbol. The remaining columns switch on
    // in the order they become affordable, so nothing important is cut off.
    let width = width.saturating_sub(2);
    let show_version = width >= 41;
    let show_status = width >= 49;
    let show_id = width >= 80;

    // What the row will be, against what the project has on disk: a change
    // nobody has saved yet is the thing to see first.
    let (mark, mark_style) = match (app.recorded(row).is_some(), row.enabled) {
        (false, true) => ("+", theme.warn_style()),
        (true, false) => ("-", theme.warn_style()),
        (_, true) => ("✓", theme.ok_style()),
        (_, false) => ("·", theme.dim_style()),
    };

    let mut spans = vec![
        Span::styled(format!("{mark} "), mark_style),
        Span::raw(fit(&model.display_name, 22)),
    ];
    // The marker comes before the optional columns: a template, or a model
    // this deployment added itself, must be recognisable even in a pane too
    // narrow for anything else. An entry is one or the other, never both.
    if model.is_template() {
        spans.push(Span::styled(" [template]", theme.dim_style()));
    } else if model.manual {
        spans.push(Span::styled(" [manual]", theme.dim_style()));
    }
    if show_id {
        spans.push(Span::styled(
            format!(" {}", fit(&model.id, 30)),
            theme.dim_style(),
        ));
    }
    if show_status {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit(status_label(model.assessed_status), 7),
            theme.status_style(model.assessed_status),
        ));
    }
    if show_version {
        // A manual entry's version is its image tag, so `v` in front of it
        // would read as a version number it does not have.
        let version = app
            .resolved(row)
            .map(|v| match model.manual {
                true => v.version.clone(),
                false => format!("v{}", v.version),
            })
            .unwrap_or_else(|| "-".to_string());
        spans.push(Span::styled(
            format!(" {}", fit(&version, 8)),
            theme.dim_style(),
        ));
    }
    // Enabled rows say how they are reached: their own host port, or that they
    // are only on the compose network. It follows the pending publish flag, so
    // pressing `p` is visible before saving - the port itself is only picked
    // when the selection is applied, hence `:auto`.
    if row.enabled {
        let (text, style) = match (row.publish, row.port) {
            (true, Some(port)) => (format!(" :{port}"), theme.accent_style()),
            (true, None) => (" :auto".to_string(), theme.warn_style()),
            (false, _) => (" internal".to_string(), theme.dim_style()),
        };
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

fn draw_details(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = pane(theme, "Details", false);
    let Some(row) = app.selected() else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "nothing matches this filter",
                theme.dim_style(),
            ))
            .block(block),
            area,
        );
        return;
    };

    let model = app.model(row);
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        model.display_name.clone(),
        theme.accent_style(),
    )));
    lines.push(Line::from(Span::styled(
        model.id.clone(),
        theme.dim_style(),
    )));

    // What this project already runs outranks the catalogue blurb: a short
    // pane only shows the top of the paragraph.
    if let Some(recorded) = app.recorded(row) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "enabled in this project",
            theme.ok_style(),
        )));
        lines.push(field(
            theme,
            "reach",
            &match recorded.host_port {
                Some(port) => format!("http://localhost:{port}"),
                None => "internal (through chap-core's /run/ proxy)".to_string(),
            },
        ));
        lines.push(field(
            theme,
            "pinned",
            // The version and the tag are the same thing for a manually
            // added model; printing it twice says nothing twice.
            &match recorded.version == recorded.image_tag {
                true => recorded.image_tag.clone(),
                false => format!("{} ({})", recorded.version, recorded.image_tag),
            },
        ));
        lines.push(field(theme, "data dir", &recorded.data_dir));
        lines.push(field(theme, "user", &recorded.user));
    }

    lines.push(Line::raw(""));
    lines.push(Line::raw(model.summary.clone()));
    lines.push(Line::raw(""));

    // A template is not a forecasting model, and the pane says so in a colour
    // that is neither "good" nor "bad", just different.
    lines.push(Line::from(vec![
        Span::styled(fit("kind", LABEL_WIDTH), theme.label_style()),
        if model.is_template() {
            Span::styled(
                "template (scaffolding, not a forecasting model)",
                theme.template_style(),
            )
        } else if model.manual {
            Span::styled("manual (added with `chaps models add`)", theme.dim_style())
        } else {
            Span::raw("model")
        },
    ]));
    lines.push(Line::from(vec![
        Span::styled(fit("status", LABEL_WIDTH), theme.label_style()),
        Span::styled(
            status_label(model.assessed_status).to_string(),
            theme.status_style(model.assessed_status),
        ),
    ]));
    lines.push(field(theme, "channel", channel_label(row.channel)));

    match app.resolved(row) {
        Some(version) => {
            lines.push(field(
                theme,
                "image",
                &crate::compose::image_ref(&model.source.image, &version.image_tag),
            ));
            lines.push(field(
                theme,
                "version",
                &format!("{} ({})", version.version, version_status(version)),
            ));
        }
        None => lines.push(field(
            theme,
            "image",
            "unresolved: this channel has no version",
        )),
    }

    let runtime = if model.needs_amd64() {
        format!("{} (amd64 only)", model.source.runtime_image)
    } else {
        model.source.runtime_image.clone()
    };
    lines.push(field(theme, "runtime", &runtime));
    lines.push(field(
        theme,
        "periods",
        &model.compatibility.period_types.join(", "),
    ));
    lines.push(field(
        theme,
        "horizon",
        &format!(
            "{} to {} periods",
            model.compatibility.min_prediction_periods, model.compatibility.max_prediction_periods
        ),
    ));
    lines.push(field(
        theme,
        "geo",
        if model.compatibility.requires_geo {
            "polygons required"
        } else {
            "not required"
        },
    ));
    lines.push(field(
        theme,
        "covariates",
        &list_or_dash(&model.covariates.required),
    ));
    lines.push(field(
        theme,
        "defaults",
        &list_or_dash(&model.covariates.defaults),
    ));

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("versions", theme.label_style())));
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
                theme.version_style(version.status),
            ),
            Span::styled(marker, theme.accent_style()),
        ]));
    }

    if !model.maintainers.is_empty() {
        lines.push(Line::raw(""));
        lines.push(field(theme, "maintainers", &model.maintainers.join(", ")));
    }
    lines.push(field(theme, "author", &model.attribution.author));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let line = if let Some(message) = &app.message {
        Line::from(Span::styled(format!(" {message}"), theme.warn_style()))
    } else if app.mode == Mode::Filter {
        let mut spans = vec![
            Span::styled(" filter: ", theme.accent_style()),
            Span::styled(app.filter.clone(), theme.label_style()),
            Span::styled("_   ", theme.accent_style()),
        ];
        spans.extend(keybar(Mode::Filter, theme));
        Line::from(spans)
    } else {
        let mut spans = Vec::new();
        if !app.filter.is_empty() {
            spans.push(Span::styled(
                format!(" filter: {}  ", app.filter),
                theme.accent_style(),
            ));
        } else {
            spans.push(Span::raw(" "));
        }
        spans.extend(keybar(app.mode, theme));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// The key bar for a mode: the keys themselves in the accent, what they do in
/// the quiet colour, so the line reads as keys first.
fn keybar<'a>(mode: Mode, theme: &Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    for (i, (key, what)) in keys::keybar_entries(mode).iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled((*key).to_string(), theme.accent_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled((*what).to_string(), theme.dim_style()));
    }
    spans
}

fn draw_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let entries = keys::help_entries();
    let width = 52u16;
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
fn pane<'a>(theme: &Theme, title: &'a str, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(Span::styled(
            format!(" {title} "),
            theme.pane_title_style(focused),
        ))
}

/// An overlay: the same shape as a pane, always in the accent.
fn overlay<'a>(theme: &Theme, title: &'a str) -> Block<'a> {
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

fn status_label(status: crate::registry::AssessedStatus) -> &'static str {
    use crate::registry::AssessedStatus;
    match status {
        AssessedStatus::Green => "green",
        AssessedStatus::Yellow => "yellow",
        AssessedStatus::Orange => "orange",
        AssessedStatus::Red => "red",
        AssessedStatus::Gray => "gray",
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

    #[test]
    fn a_normal_terminal_shows_the_title_list_and_details() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let screen = render(&app, 140, 40);
        assert!(screen.contains("chaps · models"));
        assert!(screen.contains("registry: embedded"));
        assert!(screen.contains("6 models, 0 enabled, 0 pending changes"));
        assert!(screen.contains("Marketplace"));
        assert!(screen.contains("Details"));
        // Rounded corners, not square ones.
        assert!(screen.contains('╭') && screen.contains('╯'), "{screen}");
        assert!(screen.contains("CHAP-EWARS"));
        assert!(screen.contains("chapkit_ewars_model"));
        assert!(screen.contains("space toggle"));
        // The details pane describes the row under the cursor.
        assert!(screen.contains("verified"));
        assert!(screen.contains("amd64"));
    }

    /// A project state with `chapkit_ewars_model` enabled on `host_port`.
    fn state_with_ewars(registry: &Registry, host_port: Option<u16>) -> ProjectState {
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
                host_port,
                data_dir: "/app/data".into(),
                user: "chapkit:chapkit".into(),
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
        let screen = render(&app, 100, 30);
        assert!(screen.contains('✓'), "{screen}");
        assert!(screen.contains(":5001"));
        assert!(screen.contains("enabled in this project"));
        assert!(screen.contains("http://localhost:5001"));
    }

    #[test]
    fn a_pending_change_is_marked_before_it_is_saved() {
        let registry = registry();
        let state = state_with_ewars(&registry, Some(5001));
        let mut app = App::new(&registry, &state);

        // The first row is the one the project already runs; turning it off is
        // a removal, not an absence.
        app.reduce(Action::Toggle);
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, 120, &theme());
        assert_eq!(line.spans[0].content, "- ");
        assert_eq!(line.spans[0].style, theme().warn_style());

        // And back on again is no change at all.
        app.reduce(Action::Toggle);
        let row = app.selected().expect("a row is selected");
        assert_eq!(row_line(&app, row, 120, &theme()).spans[0].content, "✓ ");

        // A row the project never had reads as an addition.
        let mut fresh = App::new(&registry, &ProjectState::default());
        fresh.reduce(Action::Toggle);
        let row = fresh.selected().expect("a row is selected");
        let line = row_line(&fresh, row, 120, &theme());
        assert_eq!(line.spans[0].content, "+ ");
        assert_eq!(line.spans[0].style, theme().warn_style());
    }

    #[test]
    fn a_row_nobody_enabled_is_marked_with_a_quiet_dot() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, 120, &theme());
        assert_eq!(line.spans[0].content, "· ");
        assert_eq!(line.spans[0].style, theme().dim_style());
    }

    #[test]
    fn the_status_column_carries_the_assessment_colour() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");
        let line = row_line(&app, row, 120, &theme());
        let status = line
            .spans
            .iter()
            .find(|s| s.content.trim() == "orange")
            .expect("the status column is there");
        assert_eq!(status.style, theme().status_style(row_status(&app, row)));
        assert_eq!(status.style.fg, Some(theme().warn));
    }

    fn row_status(app: &App, row: &Row) -> crate::registry::AssessedStatus {
        app.model(row).assessed_status
    }

    #[test]
    fn an_enabled_model_with_no_host_port_reads_as_internal() {
        let registry = registry();
        let state = state_with_ewars(&registry, None);
        let mut app = App::new(&registry, &state);
        let screen = render(&app, 100, 30);
        assert!(screen.contains('✓'));
        assert!(screen.contains("internal"), "{screen}");
        assert!(!screen.contains(":500"), "no host port is published");
        assert!(screen.contains("proxy"), "the details name the way in");

        // Pressing p is visible before saving, with the port left to apply.
        app.reduce(Action::TogglePublish);
        let screen = render(&app, 100, 30);
        assert!(screen.contains(":auto"), "{screen}");
    }

    #[test]
    fn a_row_nobody_enabled_says_nothing_about_ports() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        let row = app.selected().expect("a row is selected");
        let text = line_text(&row_line(&app, row, 120, &theme()));
        assert!(!text.contains("internal"), "{text}");
        assert!(!text.contains(':'), "{text}");
    }

    #[test]
    fn templates_are_marked_when_shown() {
        let registry = registry();
        let mut app = App::new(&registry, &ProjectState::default());
        assert!(!render(&app, 100, 30).contains("[template]"));
        app.reduce(Action::ToggleTemplates);
        assert!(render(&app, 100, 30).contains("[template]"));
    }

    /// A model the deployment added itself is marked in the list and named
    /// as such in the details, so the browser never presents a local
    /// definition as a reviewed marketplace entry.
    #[test]
    fn a_manually_added_model_is_marked_and_named() {
        let mut state = ProjectState::default();
        state.manual.insert(
            "chapkit_ghr_model".to_string(),
            crate::project::ManualModel {
                service_id: "chapkit-ghr-model".into(),
                display_name: "chapkit_ghr_model".into(),
                repository: Some("https://github.com/chap-models/chapkit_ghr_model".into()),
                image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
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
        let screen = render(&app, 100, 30);
        assert!(screen.contains("[manual]"), "{screen}");
        assert!(!screen.contains("[template]"), "{screen}");

        // The details pane of that row says what kind of entry it is.
        while app.selected().map(|row| app.model(row).id.as_str()) != Some("chapkit_ghr_model") {
            app.reduce(Action::Down);
        }
        let screen = render(&app, 100, 30);
        assert!(screen.contains("manual (added with `chaps"), "{screen}");
        // The row shows the tag as the tag, not as a version number.
        assert!(screen.contains("sha-b1"), "{screen}");
        assert!(!screen.contains("vsha-"), "{screen}");
        assert!(
            screen.contains("ghcr.io/chap-models/chapkit_ghr_model:sha-"),
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
        assert!(screen.contains("Esc clear"), "{screen}");
    }

    #[test]
    fn the_key_bar_changes_with_the_mode_and_paints_the_keys() {
        let theme = theme();
        let browse = keybar(Mode::Browse, &theme);
        assert_eq!(browse[0].content, "j/k");
        assert_eq!(browse[0].style, theme.accent_style());
        assert_eq!(browse[2].style, theme.dim_style());

        let confirm = keybar(Mode::ConfirmQuit, &theme);
        assert_eq!(confirm[0].content, "y");
        assert!(
            line_text(&Line::from(confirm)).contains("discard the changes"),
            "the bar says what the mode is about"
        );
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
    fn the_layout_holds_from_a_tiny_terminal_to_a_huge_one() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        for (width, height) in [(60, 16), (80, 24), (200, 60)] {
            let screen = render(&app, width, height);
            assert!(screen.contains("chaps · models"), "{width}x{height}");
            assert!(screen.contains("Marketplace"), "{width}x{height}");
            assert!(screen.contains("Details"), "{width}x{height}");
            assert!(screen.contains("CHAP-EWARS"), "{width}x{height}");
            assert!(screen.contains("space toggle"), "{width}x{height}");
            assert_eq!(
                screen.lines().count(),
                height as usize,
                "the frame fills the terminal exactly"
            );
        }
    }

    #[test]
    fn the_title_bar_drops_what_does_not_fit_instead_of_overflowing() {
        let registry = registry();
        let app = App::new(&registry, &ProjectState::default());
        // Wide: both halves.
        let wide = render(&app, 140, 20);
        let first = wide.lines().next().unwrap();
        assert!(first.contains("registry: embedded"));
        assert!(first.contains("6 models"));

        // Narrow: the counts are worth more than where the file came from.
        let narrow = render(&app, 60, 16);
        let first = narrow.lines().next().unwrap();
        assert_eq!(first.chars().count(), 60);
        assert!(!first.contains("registry: embedded"), "{first}");
        assert!(first.contains("chaps · models"));
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
        let line = row_line(&app, row, 120, &mono);
        assert!(line.spans.iter().all(|s| s.style.fg.is_none()));
        assert_ne!(
            line.spans[0].style,
            ratatui::style::Style::default(),
            "the tick still stands out"
        );
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

        let wide = line_text(&row_line(&app, row, 120, &theme()));
        assert!(wide.contains("CHAP-EWARS"));
        assert!(
            wide.contains("chapkit_ewars_model"),
            "the id fits when wide"
        );
        assert!(wide.contains("orange"), "the assessed status has a column");

        let narrow = line_text(&row_line(&app, row, 30, &theme()));
        assert!(narrow.contains("CHAP-EWARS"), "the name always fits");
        assert!(
            !narrow.contains("chapkit_ewars_model"),
            "the id column drops out of a narrow pane"
        );
        assert!(!narrow.contains("orange"), "so does the status column");
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
        app.reduce(Action::Help);
        for (width, height) in [(1, 1), (10, 4), (30, 6), (60, 16), (80, 24)] {
            render(&app, width, height);
        }
        app.reduce(Action::Help);
        app.reduce(Action::Toggle);
        app.reduce(Action::Quit);
        for (width, height) in [(1, 1), (10, 4), (30, 6), (60, 16), (80, 24)] {
            render(&app, width, height);
        }
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
