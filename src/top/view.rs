//! The state of the `chaps top` screen and how it is drawn.

use super::data::{Node, Service};
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Wrap};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// One line of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Node(usize),
    Service(usize, usize),
}

/// A pane of log lines under the tree.
#[derive(Debug, Clone, Default)]
pub struct Logs {
    pub title: String,
    pub lines: Vec<String>,
}

/// A stop the operator still has to confirm.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub dir: PathBuf,
    pub id: String,
}

/// Everything the screen shows.
#[derive(Debug, Default)]
pub struct App {
    pub nodes: Vec<Node>,
    /// Folded deployments, by compose project name, so a fold survives a
    /// refresh that reorders nothing.
    pub folded: BTreeSet<String>,
    pub selected: usize,
    pub message: Option<String>,
    pub confirm: Option<Confirm>,
    pub logs: Option<Logs>,
    /// When the shown snapshot was taken, as `HH:MM:SS`.
    pub refreshed: String,
    pub interval: u64,
    /// No snapshot has arrived yet.
    pub loading: bool,
}

impl App {
    /// The tree, flattened to the lines it draws.
    pub fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for (n, node) in self.nodes.iter().enumerate() {
            out.push(Entry::Node(n));
            if !self.folded.contains(&node.project) {
                out.extend((0..node.services.len()).map(|s| Entry::Service(n, s)));
            }
        }
        out
    }

    /// Put a new snapshot in, keeping the cursor on the same line when it is
    /// still there.
    pub fn replace(&mut self, nodes: Vec<Node>, refreshed: String) {
        let keep = self.selected_key();
        self.nodes = nodes;
        self.refreshed = refreshed;
        self.loading = false;
        let entries = self.entries();
        self.selected = keep
            .and_then(|key| entries.iter().position(|e| self.key_of(*e) == key))
            .unwrap_or(self.selected)
            .min(entries.len().saturating_sub(1));
    }

    fn key_of(&self, entry: Entry) -> (String, Option<String>) {
        match entry {
            Entry::Node(n) => (self.nodes[n].project.clone(), None),
            Entry::Service(n, s) => (
                self.nodes[n].project.clone(),
                Some(self.nodes[n].services[s].service.clone()),
            ),
        }
    }

    fn selected_key(&self) -> Option<(String, Option<String>)> {
        self.entries()
            .get(self.selected)
            .map(|entry| self.key_of(*entry))
    }

    pub fn selected_entry(&self) -> Option<Entry> {
        self.entries().get(self.selected).copied()
    }

    pub fn selected_service(&self) -> Option<(&Node, &Service)> {
        match self.selected_entry()? {
            Entry::Service(n, s) => Some((&self.nodes[n], &self.nodes[n].services[s])),
            Entry::Node(_) => None,
        }
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.entries().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    /// Fold or unfold the deployment under the cursor, or the one the
    /// service under it belongs to.
    pub fn toggle(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let n = match entry {
            Entry::Node(n) | Entry::Service(n, _) => n,
        };
        let project = self.nodes[n].project.clone();
        if !self.folded.remove(&project) {
            self.folded.insert(project);
        }
        self.selected = self
            .entries()
            .iter()
            .position(|e| *e == Entry::Node(n))
            .unwrap_or(0);
    }
}

/// Draw the whole screen.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let logs_height = if app.logs.is_some() {
        Constraint::Percentage(40)
    } else {
        Constraint::Length(0)
    };
    let [head, body, logs, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        logs_height,
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(header(app, theme), head);
    draw_tree(frame, app, theme, body);
    if let Some(pane) = &app.logs {
        draw_logs(frame, pane, theme, logs);
    }
    frame.render_widget(footer(app, theme), foot);
}

fn header<'a>(app: &App, theme: &Theme) -> Paragraph<'a> {
    let deployments = app.nodes.len();
    let running: usize = app.nodes.iter().map(Node::running).sum();
    let services: usize = app.nodes.iter().map(|n| n.services.len()).sum();
    let mut spans = vec![
        Span::styled(" chaps top ", theme.accent_style()),
        Span::raw(format!(
            " {deployments} deployment{}  {running}/{services} running",
            if deployments == 1 { "" } else { "s" }
        )),
    ];
    if app.loading {
        spans.push(Span::styled("  asking docker ...", theme.dim_style()));
    } else {
        spans.push(Span::styled(
            format!("  at {} (every {}s)", app.refreshed, app.interval),
            theme.dim_style(),
        ));
    }
    Paragraph::new(Line::from(spans))
}

fn footer<'a>(app: &App, theme: &Theme) -> Paragraph<'a> {
    if let Some(confirm) = &app.confirm {
        return Paragraph::new(Line::from(vec![
            Span::styled(format!(" stop {}? ", confirm.id), theme.warn_style()),
            Span::styled("y", theme.accent_style()),
            Span::raw(" yes  "),
            Span::styled("n", theme.accent_style()),
            Span::raw(" no"),
        ]));
    }
    if let Some(message) = &app.message {
        return Paragraph::new(Span::styled(format!(" {message}"), theme.warn_style()));
    }
    let keys = [
        ("j/k", "move"),
        ("enter", "fold"),
        ("l", "logs"),
        ("s", "stop model"),
        ("r", "refresh"),
        ("q", "quit"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in keys {
        spans.push(Span::styled(key, theme.accent_style()));
        spans.push(Span::styled(format!(" {what}  "), theme.dim_style()));
    }
    Paragraph::new(Line::from(spans))
}

/// The state cell's style: green when running and healthy, yellow while it
/// starts, red when it stopped or never started.
fn state_style(service: &Service, theme: &Theme) -> Style {
    match (service.state.as_str(), service.health.as_deref()) {
        ("running", Some("unhealthy")) => theme.bad_style(),
        ("running", Some("starting")) => theme.warn_style(),
        ("running", _) => theme.ok_style(),
        ("not running", _) => theme.dim_style(),
        _ => theme.bad_style(),
    }
}

fn state_text(service: &Service) -> String {
    match &service.health {
        Some(health) => format!("{} ({health})", service.state),
        None => service.state.clone(),
    }
}

fn draw_tree(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let entries = app.entries();
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| match *entry {
            Entry::Node(n) => {
                let node = &app.nodes[n];
                let marker = if app.folded.contains(&node.project) {
                    "▸"
                } else {
                    "▾"
                };
                Row::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{marker} "), theme.dim_style()),
                        Span::styled(node.name.clone(), theme.label_style()),
                        Span::styled(format!("  {}", node.kind.label()), theme.dim_style()),
                    ]),
                    Line::styled(
                        format!("{}/{} running", node.running(), node.services.len()),
                        theme.dim_style(),
                    ),
                    Line::raw(""),
                    Line::raw(""),
                    Line::styled(
                        node.dir
                            .as_ref()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default(),
                        theme.dim_style(),
                    ),
                ])
            }
            Entry::Service(n, s) => {
                let node = &app.nodes[n];
                let service = &node.services[s];
                let branch = if s + 1 == node.services.len() {
                    "  └─ "
                } else {
                    "  ├─ "
                };
                let name = match &service.model {
                    Some(id) if *id != service.service => {
                        format!("{} ({id})", service.service)
                    }
                    _ => service.service.clone(),
                };
                Row::new(vec![
                    Line::from(vec![
                        Span::styled(branch, theme.dim_style()),
                        Span::raw(name),
                    ]),
                    Line::styled(state_text(service), state_style(service, theme)),
                    Line::raw(service.cpu.clone().unwrap_or_default()),
                    Line::raw(service.memory.clone().unwrap_or_default()),
                    Line::raw(service.url.clone().unwrap_or_default()),
                ])
            }
        })
        .collect();
    let empty = rows.is_empty() && !app.loading;
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(34),
            Constraint::Length(22),
            Constraint::Length(8),
            Constraint::Length(22),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(vec![
            "DEPLOYMENT / SERVICE",
            "STATE",
            "CPU",
            "MEMORY",
            "URL",
        ])
        .style(theme.column_style()),
    )
    .row_highlight_style(theme.selection_style())
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(app.logs.is_none())),
    );
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
    if empty {
        let inner = area.inner(ratatui::layout::Margin::new(2, 2));
        frame.render_widget(
            Paragraph::new(
                "no chaps deployment has a container on this machine; \
                 `chaps run <model>` or `chaps up` starts one",
            )
            .style(theme.dim_style())
            .wrap(Wrap { trim: true }),
            inner,
        );
    }
}

fn draw_logs(frame: &mut Frame, pane: &Logs, theme: &Theme, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let start = pane.lines.len().saturating_sub(height);
    let text: Vec<Line> = pane.lines[start..]
        .iter()
        .map(|l| Line::raw(l.clone()))
        .collect();
    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border_style(true))
                .title(Span::styled(
                    format!(" {} (l or esc closes) ", pane.title),
                    theme.pane_title_style(true),
                )),
        ),
        area,
    );
}
