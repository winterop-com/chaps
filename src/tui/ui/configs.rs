//! The configured models page of one model, its form and its confirmation.

use super::dialogs::draw_dialog;
use super::text::{centered, fit, fit_soft, overlay, wrap};
use crate::configs::options;
use crate::tui::app::configs::{Load, Pending, View};
use crate::tui::app::{App, Mode, Page};
use crate::tui::form::Form;
use crate::tui::keys;
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// The width of the columns before OPTIONS.
const NAME_WIDTH: usize = 26;
const COVARIATES_WIDTH: usize = 26;
const SOURCE_WIDTH: usize = 16;

/// The page, and the form or the question on top of it.
pub(super) fn draw_configs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(view) = &app.configs else {
        return;
    };
    let width = area.width.saturating_sub(4).clamp(1, 110);
    let height = area.height.saturating_sub(4).max(3.min(area.height));
    let popup = centered(area, width, height);
    let inner = popup.width.saturating_sub(3) as usize;
    let lines: Vec<Line> = page_lines(view, inner, theme)
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            Line::from(spans).style(line.style)
        })
        .collect();
    let title = format!("Configured models · {} · {}", view.display_name, view.model);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(overlay(theme, &title)), popup);

    match (app.mode, &view.form) {
        (Mode::ConfigForm, Some(form)) => draw_form(frame, area, view, form, theme),
        (Mode::ConfigConfirm, _) => draw_question(frame, area, view, theme),
        _ => {}
    }
}

/// What the page says: the table, or why there is none.
pub(super) fn page_lines<'a>(view: &View, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let configs = match &view.load {
        Load::Loading => {
            return vec![Line::from(Span::styled(
                "asking chap-core ...",
                theme.dim_style(),
            ))];
        }
        Load::Failed(why) => {
            return wrap(why, width)
                .into_iter()
                .map(|line| Line::from(Span::styled(line, theme.warn_style())))
                .collect();
        }
        Load::Ready { configs, .. } => configs,
    };
    if configs.is_empty() {
        let text = format!(
            "chap-core has no configured model of {}; press a to add one, or run `varde models \
             configs sync {}`",
            view.model, view.model
        );
        return wrap(&text, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.dim_style())))
            .collect();
    }
    let options_width = width.saturating_sub(NAME_WIDTH + COVARIATES_WIDTH + SOURCE_WIDTH);
    let mut lines = vec![Line::from(vec![
        Span::styled(fit("NAME", NAME_WIDTH), theme.column_style()),
        Span::styled(fit("COVARIATES", COVARIATES_WIDTH), theme.column_style()),
        Span::styled(fit("SOURCE", SOURCE_WIDTH), theme.column_style()),
        Span::styled("OPTIONS", theme.column_style()),
    ])];
    for (at, (config, source)) in configs.iter().enumerate() {
        let covariates = match config.covariates.is_empty() {
            true => "-".to_string(),
            false => config.covariates.join(","),
        };
        let values = match config.values.is_empty() {
            true => "-".to_string(),
            false => options::short(&config.values, options_width.saturating_sub(1)),
        };
        let line = Line::from(vec![
            Span::raw(fit(&config.variant, NAME_WIDTH)),
            Span::styled(fit(&covariates, COVARIATES_WIDTH), theme.dim_style()),
            Span::styled(fit(source, SOURCE_WIDTH), theme.dim_style()),
            Span::styled(fit_soft(&values, options_width), theme.dim_style()),
        ]);
        lines.push(match at == view.cursor {
            true => line.style(theme.selection_style()),
            false => line,
        });
    }
    lines
}

/// The form lines: one per field, then what the field under the cursor
/// takes, then why the last save was refused.
pub(super) fn form_lines<'a>(form: &Form, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let label_width = form
        .fields
        .iter()
        .map(|field| field.label.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut lines = Vec::new();
    for (at, field) in form.fields.iter().enumerate() {
        let here = at == form.cursor;
        let mut spans = vec![
            Span::styled(if here { " ▸ " } else { "   " }, theme.accent_style()),
            Span::styled(fit(&field.label, label_width), theme.label_style()),
        ];
        // An empty field shows the default it stands for, dimmed.
        spans.push(match field.input.is_empty() {
            true => Span::styled(field.default.clone(), theme.dim_style()),
            false => Span::styled(field.input.clone(), theme.accent_style()),
        });
        if here {
            spans.push(Span::styled("_", theme.accent_style()));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    if let Some(field) = form.fields.get(form.cursor) {
        let about = match field.description.is_empty() {
            true => field.kind.clone(),
            false => format!("{}. {}", field.kind, field.description),
        };
        for line in wrap(&about, width.saturating_sub(2)) {
            lines.push(Line::from(Span::styled(
                format!(" {line}"),
                theme.dim_style(),
            )));
        }
    }
    // Always drawn, empty or not, so the keys below it never move.
    let error = form.error.clone().unwrap_or_default();
    let mut errors = wrap(&error, width.saturating_sub(2));
    if errors.is_empty() {
        errors.push(String::new());
    }
    for line in errors {
        lines.push(Line::from(Span::styled(
            format!(" {line}"),
            theme.bad_style(),
        )));
    }
    lines
}

fn draw_form(frame: &mut Frame, area: Rect, view: &View, form: &Form, theme: &Theme) {
    draw_form_dialog(frame, area, &form_title(form, &view.model), form, theme);
}

/// The title of the form: what it creates or changes.
pub fn form_title(form: &Form, model: &str) -> String {
    match &form.fixed_name {
        Some(name) => format!("Configured model {name} of {model}"),
        None => format!("New configured model of {model}"),
    }
}

/// The form as a dialog in `area`, with its own key line.
pub fn draw_form_dialog(frame: &mut Frame, area: Rect, title: &str, form: &Form, theme: &Theme) {
    let width = area.width.saturating_sub(8).clamp(1, 80) as usize;
    let lines = form_lines(form, width, theme);
    let hints = keys::keybar(Page::Models, Mode::ConfigForm, 0, false);
    draw_dialog(frame, area, title, lines, &hints, theme);
}

/// The question before chap-core is asked to write.
pub(super) fn question(view: &View) -> (String, Vec<String>) {
    match &view.pending {
        Some(Pending::Create(draft)) => {
            let mut lines = vec![format!(
                "Create the configured model {} of {} in chap-core?",
                draft.variant, view.model
            )];
            if !draft.values.is_empty() {
                lines.push(format!("options: {}", options::short(&draft.values, 70)));
            }
            if !draft.covariates.is_empty() {
                lines.push(format!("covariates: {}", draft.covariates.join(",")));
            }
            ("Create".to_string(), lines)
        }
        Some(Pending::Update { draft, .. }) => {
            let mut lines = vec![
                format!(
                    "Update the configured model {} of {} in chap-core?",
                    draft.variant, view.model
                ),
                "chap-core keeps the old values as an archived configured model.".to_string(),
            ];
            if !draft.values.is_empty() {
                lines.push(format!("options: {}", options::short(&draft.values, 70)));
            }
            lines.push(format!(
                "covariates: {}",
                match draft.covariates.is_empty() {
                    true => "-".to_string(),
                    false => draft.covariates.join(","),
                }
            ));
            ("Update".to_string(), lines)
        }
        Some(Pending::Archive { variant, .. }) => (
            "Archive".to_string(),
            vec![
                format!("Archive the configured model {variant} of {}?", view.model),
                "chap-core keeps it, and the Modeling App shows it as Archived.".to_string(),
            ],
        ),
        None => (String::new(), Vec::new()),
    }
}

fn draw_question(frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
    let (title, text) = question(view);
    let lines: Vec<Line> = text
        .into_iter()
        .map(|line| Line::from(Span::raw(format!(" {line}"))))
        .collect();
    let hints = keys::keybar(Page::Models, Mode::ConfigConfirm, 0, false);
    draw_dialog(frame, area, &title, lines, &hints, theme);
}

#[cfg(test)]
mod tests;
