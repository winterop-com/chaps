//! The shared pieces every part of the screen draws with: boxes, fields,
//! and fitting text into a width.

use crate::registry::{Channel, Version, VersionStatus};
use crate::tui::app::{App, Row};
use crate::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

/// The label column in the details overlay.
pub(super) const LABEL_WIDTH: usize = 12;

/// A pane: rounded border, a title that follows the border's colour.
pub(super) fn pane<'a>(theme: &Theme, title: &str, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(Span::styled(
            format!(" {title} "),
            theme.pane_title_style(focused),
        ))
}

/// An overlay: the same shape as a pane, always in the accent.
pub(super) fn overlay<'a>(theme: &Theme, title: &str) -> Block<'a> {
    pane(theme, title, true)
}

/// A centred rectangle that never leaves `area`, whatever the terminal size.
pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(super) fn field<'a>(theme: &Theme, label: &str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(fit(label, LABEL_WIDTH), theme.label_style()),
        Span::raw(value.to_string()),
    ])
}

/// [`field`] for a value that can be longer than the box: it wraps under its
/// own label instead of being cut off at the border.
pub(super) fn wrapped_field<'a>(
    theme: &Theme,
    label: &str,
    value: &str,
    width: usize,
) -> Vec<Line<'a>> {
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
pub(super) fn with_tail<'a>(
    mut head: Vec<Span<'a>>,
    tail: Span<'a>,
    width: usize,
) -> Vec<Span<'a>> {
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
pub(super) fn with_kept_tail<'a>(
    mut head: Vec<Span<'a>>,
    tail: Span<'a>,
    width: usize,
) -> Vec<Span<'a>> {
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
pub(super) fn truncate(spans: &mut Vec<Span>, width: usize) {
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
pub(super) fn width_of_line(line: &Line) -> usize {
    width_of(&line.spans)
}

/// How many columns a run of spans occupies.
pub(super) fn width_of(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

pub(super) fn list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

pub(super) fn horizon(app: &App, row: &Row) -> String {
    let c = &app.model(row).compatibility;
    format!(
        "horizon {} to {} periods",
        c.min_prediction_periods, c.max_prediction_periods
    )
}

/// `1.0.3 (sha-24d58c0)`, or just the tag when the two are the same thing, as
/// they are for a manually added model.
pub(super) fn version_and_tag(version: &str, tag: &str) -> String {
    if version == tag {
        tag.to_string()
    } else {
        format!("{version} ({tag})")
    }
}

/// Pad or truncate to exactly `width` characters.
pub(super) fn fit(text: &str, width: usize) -> String {
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
pub(super) fn fit_soft(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    fit(text, width)
}

/// Break text into lines of at most `width` columns, on spaces where there is
/// one and mid-word where there is not.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
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

pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub(super) fn channel_label(channel: Channel) -> &'static str {
    channel.as_str()
}

pub(super) fn version_status(version: &Version) -> &'static str {
    match version.status {
        VersionStatus::Verified => "verified",
        VersionStatus::Unstable => "unstable",
        VersionStatus::Deprecated => "deprecated",
        VersionStatus::Yanked => "yanked",
    }
}
