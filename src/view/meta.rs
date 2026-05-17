//! Cluster Metadata Store TUI view — Phase 37.
//!
//! Two sub-views:
//!
//! | Sub-view | Trigger | Description |
//! |----------|---------|-------------|
//! | `DateList` | `:meta` / `:cmeta` | Date index on left, record list on right |
//! | `Detail`   | `Enter` on record | Scrollable prettified JSON of a single record |
//!
//! Filter bar (`f` key) cycles: All → Snapshot → Issue → Interaction → All.

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};
use ratatui::Frame;

use crate::meta::MetadataRecord;
use crate::render::meta as rmeta;

// ─── Filter ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum TypeFilter {
    All,
    Snapshot,
    Issue,
    Interaction,
}

impl TypeFilter {
    fn label(&self) -> &'static str {
        match self {
            TypeFilter::All => "All",
            TypeFilter::Snapshot => "Snapshot",
            TypeFilter::Issue => "Issue",
            TypeFilter::Interaction => "Interaction",
        }
    }

    fn next(&self) -> Self {
        match self {
            TypeFilter::All => TypeFilter::Snapshot,
            TypeFilter::Snapshot => TypeFilter::Issue,
            TypeFilter::Issue => TypeFilter::Interaction,
            TypeFilter::Interaction => TypeFilter::All,
        }
    }

    fn matches(&self, record: &MetadataRecord) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Snapshot => matches!(record, MetadataRecord::Snapshot(_)),
            TypeFilter::Issue => matches!(record, MetadataRecord::Issue(_)),
            TypeFilter::Interaction => matches!(record, MetadataRecord::Interaction(_)),
        }
    }
}

// ─── Sub-view ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum SubView {
    DateList,
    Detail,
}

// ─── Actions ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MetaAction {
    /// User closed the view.
    Close,
    /// Reload records for the selected date.
    LoadDate(NaiveDate),
    /// Inject the selected record as extra context into AI chat.
    InjectToChat(String),
    /// Prune records older than the retention policy.
    Prune,
    /// No action needed.
    None,
}

// ─── ClusterMetaView ──────────────────────────────────────────────────────────

/// TUI view for browsing the cluster metadata journal.
pub struct ClusterMetaView {
    sub: SubView,

    // ── Date list ─────────────────────────────────────────────────────────────
    dates: Vec<NaiveDate>,
    date_list: ListState,

    // ── Record table ──────────────────────────────────────────────────────────
    records: Vec<MetadataRecord>,
    /// Records after applying the current filter.
    filtered: Vec<usize>,
    record_table: TableState,
    filter: TypeFilter,

    // ── Detail pane ───────────────────────────────────────────────────────────
    detail_content: String,
    detail_scroll: usize,

    // ── Context ───────────────────────────────────────────────────────────────
    selected_date: Option<NaiveDate>,
    pub status: Option<String>,
}

impl ClusterMetaView {
    pub fn new() -> Self {
        Self {
            sub: SubView::DateList,
            dates: Vec::new(),
            date_list: ListState::default(),
            records: Vec::new(),
            filtered: Vec::new(),
            record_table: TableState::default(),
            filter: TypeFilter::All,
            detail_content: String::new(),
            detail_scroll: 0,
            selected_date: None,
            status: None,
        }
    }

    // ── Data setters ──────────────────────────────────────────────────────────

    pub fn set_dates(&mut self, dates: Vec<NaiveDate>) {
        self.dates = dates;
        if !self.dates.is_empty() && self.date_list.selected().is_none() {
            // Select today's date if present, otherwise the most recent.
            let today = chrono::Utc::now().date_naive();
            let idx = self
                .dates
                .iter()
                .rposition(|d| *d == today)
                .unwrap_or(self.dates.len() - 1);
            self.date_list.select(Some(idx));
            self.selected_date = Some(self.dates[idx]);
        }
        self.status = None;
    }

    pub fn set_records(&mut self, records: Vec<MetadataRecord>, date: NaiveDate) {
        self.selected_date = Some(date);
        self.records = records;
        self.apply_filter();
        self.status = None;
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, event: &KeyEvent) -> MetaAction {
        match self.sub {
            SubView::DateList => self.handle_date_list_key(event),
            SubView::Detail => self.handle_detail_key(event),
        }
    }

    fn handle_date_list_key(&mut self, event: &KeyEvent) -> MetaAction {
        match event.code {
            KeyCode::Char('q') | KeyCode::Esc => return MetaAction::Close,

            // Left/Right or h/l — navigate between date list and record table.
            // j/k or Up/Down navigate within the focused pane.
            KeyCode::Up | KeyCode::Char('k') => {
                // Move record table if dates are loaded, else move date list.
                if !self.filtered.is_empty() {
                    let i = self.record_table.selected().unwrap_or(0);
                    if i > 0 {
                        self.record_table.select(Some(i - 1));
                    }
                } else {
                    self.move_date_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.filtered.is_empty() {
                    let cur = self.record_table.selected().unwrap_or(0);
                    if cur + 1 < self.filtered.len() {
                        self.record_table.select(Some(cur + 1));
                    }
                } else {
                    self.move_date_down();
                }
            }

            // Left / H — move focus back to date list.
            KeyCode::Left | KeyCode::Char('H') => {
                self.move_date_up();
            }
            // Right / L — move focus to next date.
            KeyCode::Right | KeyCode::Char('L') => {
                self.move_date_down();
            }

            // Tab — move between date list and record table when both loaded.
            KeyCode::Tab if !self.dates.is_empty() => {
                self.move_date_down();
            }

            // Enter — open detail for selected record, or load selected date.
            KeyCode::Enter => {
                if !self.filtered.is_empty() {
                    if let Some(sel) = self.record_table.selected() {
                        if let Some(&idx) = self.filtered.get(sel) {
                            if let Some(record) = self.records.get(idx) {
                                let json = serde_json::to_string_pretty(record)
                                    .unwrap_or_else(|e| format!("serialization error: {e}"));
                                self.detail_content = json;
                                self.detail_scroll = 0;
                                self.sub = SubView::Detail;
                            }
                        }
                    }
                } else if let Some(date) = self.selected_date {
                    return MetaAction::LoadDate(date);
                }
            }

            // d — load records for selected date.
            KeyCode::Char('d') => {
                if let Some(date) = self.selected_date {
                    return MetaAction::LoadDate(date);
                }
            }

            // f — cycle filter.
            KeyCode::Char('f') => {
                self.filter = self.filter.next();
                self.apply_filter();
            }

            // c — inject selected record into AI chat.
            KeyCode::Char('c') => {
                if let Some(sel) = self.record_table.selected() {
                    if let Some(&idx) = self.filtered.get(sel) {
                        if let Some(record) = self.records.get(idx) {
                            let context = format!(
                                "Cluster history record ({}):\n{}",
                                record.type_label(),
                                record.summary()
                            );
                            return MetaAction::InjectToChat(context);
                        }
                    }
                }
            }

            // p — prune.
            KeyCode::Char('p') => return MetaAction::Prune,

            // r / F5 — refresh.
            KeyCode::Char('r') | KeyCode::F(5) => {
                if let Some(date) = self.selected_date {
                    return MetaAction::LoadDate(date);
                }
            }

            _ => {}
        }
        MetaAction::None
    }

    fn handle_detail_key(&mut self, event: &KeyEvent) -> MetaAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::DateList;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.detail_scroll += 1;
            }
            KeyCode::PageUp => {
                self.detail_scroll = self.detail_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.detail_scroll += 10;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.detail_scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.detail_scroll = usize::MAX / 2;
            }
            _ => {}
        }
        MetaAction::None
    }

    // ── Render ────────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match &self.sub {
            SubView::DateList => self.render_date_list(frame, area),
            SubView::Detail => self.render_detail(frame, area),
        }
    }

    fn render_date_list(&mut self, frame: &mut Frame, area: Rect) {
        // Split: left date pane (20%) | right record table (80%).
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
            .split(area);

        self.render_dates_pane(frame, chunks[0]);
        self.render_records_pane(frame, chunks[1]);

        // Status line at bottom of right pane.
        if let Some(msg) = &self.status {
            let hint_area = Rect {
                x: chunks[1].x + 1,
                y: chunks[1].y + chunks[1].height.saturating_sub(1),
                width: chunks[1].width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Yellow)),
                hint_area,
            );
        }
    }

    fn render_dates_pane(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .dates
            .iter()
            .rev() // Most recent first.
            .map(|d| ListItem::new(d.format("%Y-%m-%d").to_string()))
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Dates ")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray),
            )
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, area, &mut self.date_list);
    }

    fn render_records_pane(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let header_row = Row::new(
            rmeta::headers()
                .iter()
                .map(|h| Cell::from(*h).style(header_style)),
        )
        .height(1);

        let rows: Vec<Row> = self
            .filtered
            .iter()
            .filter_map(|&idx| self.records.get(idx))
            .map(|record| {
                let row = rmeta::render(record);
                let type_style = rmeta::type_color(&row.cells[1]);
                Row::new(vec![
                    Cell::from(row.cells[0].clone()),
                    Cell::from(row.cells[1].clone()).style(type_style),
                    Cell::from(row.cells[2].clone()),
                ])
            })
            .collect();

        let date_label = self
            .selected_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();

        let title = format!(
            " Cluster Metadata — {} — {} records  [f: filter={}] ",
            date_label,
            self.filtered.len(),
            self.filter.label()
        );

        let widths = [
            Constraint::Length(10),
            Constraint::Length(13),
            Constraint::Percentage(75),
        ];

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.record_table);
    }

    fn render_detail(&mut self, frame: &mut Frame, area: Rect) {
        let lines: Vec<ratatui::text::Line> = self
            .detail_content
            .lines()
            .map(ratatui::text::Line::raw)
            .collect();
        let total = lines.len();
        let visible = area.height.saturating_sub(2) as usize;
        let max_scroll = total.saturating_sub(visible);
        self.detail_scroll = self.detail_scroll.min(max_scroll);

        let para = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Record Detail (Esc to return) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .scroll((self.detail_scroll as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(para, area);

        if total > visible {
            let mut sb = ScrollbarState::new(max_scroll).position(self.detail_scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut sb,
            );
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn apply_filter(&mut self) {
        self.filtered = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| self.filter.matches(r))
            .map(|(i, _)| i)
            .collect();

        // Reset table selection.
        self.record_table = TableState::default();
        if !self.filtered.is_empty() {
            self.record_table.select(Some(0));
        }
    }

    fn move_date_up(&mut self) {
        let i = self.date_list.selected().unwrap_or(0);
        if i > 0 {
            let new = i - 1;
            self.date_list.select(Some(new));
            // dates are displayed reversed, so index 0 = most recent.
            let real_idx = self.dates.len() - 1 - new;
            self.selected_date = self.dates.get(real_idx).copied();
        }
    }

    fn move_date_down(&mut self) {
        let cur = self.date_list.selected().unwrap_or(0);
        let max = self.dates.len().saturating_sub(1);
        if cur < max {
            let new = cur + 1;
            self.date_list.select(Some(new));
            let real_idx = self.dates.len() - 1 - new;
            self.selected_date = self.dates.get(real_idx).copied();
        }
    }
}

impl Default for ClusterMetaView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::record::{IssueRecord, NodeSummary, SnapshotRecord, WorkloadSummary};
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_snapshot() -> MetadataRecord {
        MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary {
                total: 2,
                ready: 2,
                not_ready: 0,
            },
            vec!["default".into()],
            WorkloadSummary {
                deployments: 1,
                running: 1,
                degraded: 0,
            },
            "1.30.0",
        ))
    }

    fn make_issue() -> MetadataRecord {
        MetadataRecord::Issue(IssueRecord::new(
            "CrashLoopBackOff",
            "prod",
            "api",
            "Pod",
            "exited",
        ))
    }

    #[test]
    fn new_starts_in_date_list_mode() {
        let view = ClusterMetaView::new();
        assert!(matches!(view.sub, SubView::DateList));
    }

    #[test]
    fn q_returns_close() {
        let mut view = ClusterMetaView::new();
        let action = view.handle_key(&press(KeyCode::Char('q')));
        assert_eq!(action, MetaAction::Close);
    }

    #[test]
    fn esc_in_detail_returns_to_date_list() {
        let mut view = ClusterMetaView::new();
        view.sub = SubView::Detail;
        view.handle_key(&press(KeyCode::Esc));
        assert!(matches!(view.sub, SubView::DateList));
    }

    #[test]
    fn set_records_populates_filtered() {
        let mut view = ClusterMetaView::new();
        let date = chrono::Utc::now().date_naive();
        view.set_records(vec![make_snapshot(), make_issue()], date);
        assert_eq!(view.records.len(), 2);
        assert_eq!(view.filtered.len(), 2);
    }

    #[test]
    fn filter_cycles_on_f() {
        let mut view = ClusterMetaView::new();
        assert!(matches!(view.filter, TypeFilter::All));
        view.handle_key(&press(KeyCode::Char('f')));
        assert!(matches!(view.filter, TypeFilter::Snapshot));
        view.handle_key(&press(KeyCode::Char('f')));
        assert!(matches!(view.filter, TypeFilter::Issue));
        view.handle_key(&press(KeyCode::Char('f')));
        assert!(matches!(view.filter, TypeFilter::Interaction));
        view.handle_key(&press(KeyCode::Char('f')));
        assert!(matches!(view.filter, TypeFilter::All));
    }

    #[test]
    fn filter_snapshot_hides_issues() {
        let mut view = ClusterMetaView::new();
        let date = chrono::Utc::now().date_naive();
        view.set_records(vec![make_snapshot(), make_issue()], date);
        view.filter = TypeFilter::Snapshot;
        view.apply_filter();
        assert_eq!(view.filtered.len(), 1);
    }

    #[test]
    fn enter_on_record_opens_detail() {
        let mut view = ClusterMetaView::new();
        let date = chrono::Utc::now().date_naive();
        view.set_records(vec![make_snapshot()], date);
        view.handle_key(&press(KeyCode::Enter));
        assert!(matches!(view.sub, SubView::Detail));
        assert!(!view.detail_content.is_empty());
    }

    #[test]
    fn prune_returns_prune_action() {
        let mut view = ClusterMetaView::new();
        let action = view.handle_key(&press(KeyCode::Char('p')));
        assert_eq!(action, MetaAction::Prune);
    }

    #[test]
    fn set_dates_selects_last() {
        let mut view = ClusterMetaView::new();
        let d1 = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        view.set_dates(vec![d1, d2]);
        assert!(view.date_list.selected().is_some());
    }

    #[test]
    fn c_on_record_returns_inject_action() {
        let mut view = ClusterMetaView::new();
        let date = chrono::Utc::now().date_naive();
        view.set_records(vec![make_issue()], date);
        let action = view.handle_key(&press(KeyCode::Char('c')));
        assert!(matches!(action, MetaAction::InjectToChat(_)));
    }
}
