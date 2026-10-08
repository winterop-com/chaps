//! TUI rendering: title bar, the model or component table, the summary strip,
//! key bar and the overlays.
//!
//! Drawing never mutates the [`App`] except for one thing it alone can know:
//! how far the details overlay can scroll, which depends on the size of the
//! terminal. Everything else on screen is derived from the state the reducer
//! produced, and every colour comes from the [`Theme`]. The layout degrades on
//! narrow terminals by dropping columns rather than wrapping or panicking.

mod configs;
mod dialogs;
mod info;
mod strip;
mod table;
mod text;

use crate::components::Component;
use crate::registry::Provenance;
use crate::tui::app::{App, Mode, Page};
use crate::tui::keys::{self, Hint};
use crate::tui::theme::Theme;
use dialogs::{
    draw_channel_dialog, draw_confirm, draw_dhis2_version_dialog, draw_help, draw_palette,
    draw_port_dialog,
};
use info::draw_info;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use strip::{draw_strip, strip_height};
use table::{
    columns, component_columns, draw_component_head, draw_component_list, draw_head, draw_list,
    draw_rule, kind_column,
};
use text::{fit_soft, pane, width_of};

/// Draw the configured model form alone, as `varde models configs add` and
/// `update` show it at a terminal.
pub fn draw_form(frame: &mut Frame, model: &str, form: &crate::tui::form::Form, theme: &Theme) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let title = configs::form_title(form, model);
    configs::draw_form_dialog(frame, area, &title, form, theme);
}

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
        Mode::Configs | Mode::ConfigForm | Mode::ConfigConfirm => {
            configs::draw_configs(frame, area, app, theme)
        }
        _ => {}
    }
}

/// `varde · models` on the left, where the catalogue came from and how big it
/// is on the right. Both halves shrink out of the way before they collide.
fn draw_title(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let counts = app.counts();
    let mut left = vec![
        Span::styled("varde", theme.accent_style()),
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

#[cfg(test)]
mod tests;
