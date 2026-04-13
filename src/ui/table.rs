//! Sortable, scrollable table widget backed by a `Vec<Row>`.
//!
//! This is the core display primitive for all resource list views.
//! It wraps ratatui's `Table` with:
//!   - Stateful cursor / selection tracking
//!   - Column sort (ascending / descending toggle)
//!   - Row filtering by a text substring
//!   - Delta markers: Added / Modified / Deleted rows highlighted differently

use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use ratatui::Frame;

/// A single displayable column.
#[derive(Debug, Clone)]
pub struct Column {
    pub header: &'static str,
    pub width: Constraint,
}

/// Change state of a table row since last refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowDelta {
    #[default]
    Unchanged,
    Added,
    Modified,
    Deleted,
}

/// A single table row with display cells and metadata.
#[derive(Debug, Clone)]
pub struct TableRow {
    /// Display cells, one per column.
    pub cells: Vec<String>,
    /// Change state since last refresh.
    pub delta: RowDelta,
    /// Age string (used for sorting by time).
    pub age_secs: u64,
}

impl TableRow {
    pub fn new(cells: Vec<String>) -> Self {
        Self {
            cells,
            delta: RowDelta::default(),
            age_secs: 0,
        }
    }
}

/// Sort direction for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDir {
    #[default]
    Ascending,
    Descending,
}

impl SortDir {
    fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// State for the resource table widget.
pub struct TableWidget {
    columns: Vec<Column>,
    all_rows: Vec<TableRow>,
    /// Indices into `all_rows` after filtering.
    filtered_indices: Vec<usize>,
    pub state: TableState,
    sort_col: Option<usize>,
    sort_dir: SortDir,
    filter: String,
}

impl TableWidget {
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            all_rows: Vec::new(),
            filtered_indices: Vec::new(),
            state: TableState::default(),
            sort_col: None,
            sort_dir: SortDir::default(),
            filter: String::new(),
        }
    }

    /// Replace the full row set and reapply filter / sort.
    pub fn set_rows(&mut self, rows: Vec<TableRow>) {
        self.all_rows = rows;
        self.refilter();
        self.resort();
    }

    /// Return the currently selected row, if any.
    pub fn selected_row(&self) -> Option<&TableRow> {
        let idx = self.state.selected()?;
        let raw_idx = *self.filtered_indices.get(idx)?;
        self.all_rows.get(raw_idx)
    }

    /// Return the raw (pre-filter) index of the selected row.
    ///
    /// Used by `BrowserView::selected_value()` to look up the original JSON.
    pub fn selected_raw_idx(&self) -> Option<usize> {
        let idx = self.state.selected()?;
        self.filtered_indices.get(idx).copied()
    }

    /// Move cursor up by one.
    pub fn up(&mut self) {
        let len = self.filtered_indices.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        self.state
            .select(Some(if i == 0 { len - 1 } else { i - 1 }));
    }

    /// Move cursor down by one.
    pub fn down(&mut self) {
        let len = self.filtered_indices.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        self.state.select(Some((i + 1) % len));
    }

    pub fn page_up(&mut self, page_size: usize) {
        let len = self.filtered_indices.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        self.state.select(Some(i.saturating_sub(page_size)));
    }

    pub fn page_down(&mut self, page_size: usize) {
        let len = self.filtered_indices.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        self.state.select(Some((i + page_size).min(len - 1)));
    }

    pub fn top(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.state.select(Some(0));
        }
    }

    pub fn bottom(&mut self) {
        let len = self.filtered_indices.len();
        if len > 0 {
            self.state.select(Some(len - 1));
        }
    }

    /// Set a filter string. Empty string shows all rows.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.refilter();
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Toggle sort on a column index.
    pub fn sort_by_column(&mut self, col: usize) {
        if self.sort_col == Some(col) {
            self.sort_dir = self.sort_dir.toggle();
        } else {
            self.sort_col = Some(col);
            self.sort_dir = SortDir::Ascending;
        }
        self.resort();
    }

    pub fn row_count(&self) -> usize {
        self.filtered_indices.len()
    }

    fn refilter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.all_rows.len()).collect();
        } else {
            let f = self.filter.to_lowercase();
            self.filtered_indices = self
                .all_rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.cells.iter().any(|c| c.to_lowercase().contains(&f)))
                .map(|(i, _)| i)
                .collect();
        }

        // Keep selection in bounds.
        if let Some(sel) = self.state.selected() {
            if sel >= self.filtered_indices.len() {
                let new = self.filtered_indices.len().saturating_sub(1);
                self.state.select(if self.filtered_indices.is_empty() {
                    None
                } else {
                    Some(new)
                });
            }
        } else if !self.filtered_indices.is_empty() {
            self.state.select(Some(0));
        }
    }

    fn resort(&mut self) {
        let Some(col) = self.sort_col else { return };
        let rows = &self.all_rows;
        let dir = self.sort_dir;

        self.filtered_indices.sort_by(|&a, &b| {
            let ca = rows[a].cells.get(col).map(String::as_str).unwrap_or("");
            let cb = rows[b].cells.get(col).map(String::as_str).unwrap_or("");
            let ord = ca.cmp(cb);
            if dir == SortDir::Descending {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    /// Render the table into the given area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, title: &str) {
        let header_cells: Vec<Cell> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let mut label = col.header.to_owned();
                if self.sort_col == Some(i) {
                    label = match self.sort_dir {
                        SortDir::Ascending => format!("{} ▲", label),
                        SortDir::Descending => format!("{} ▼", label),
                    };
                }
                Cell::from(label).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();

        let header = Row::new(header_cells).height(1);

        let rows: Vec<Row> = self
            .filtered_indices
            .iter()
            .map(|&i| {
                let row = &self.all_rows[i];
                let style = match row.delta {
                    RowDelta::Added => Style::default().fg(Color::Green),
                    RowDelta::Modified => Style::default().fg(Color::Yellow),
                    RowDelta::Deleted => {
                        Style::default().fg(Color::Red).add_modifier(Modifier::DIM)
                    }
                    RowDelta::Unchanged => Style::default(),
                };
                let cells: Vec<Cell> = row.cells.iter().map(|c| Cell::from(c.clone())).collect();
                Row::new(cells).style(style)
            })
            .collect();

        let widths: Vec<Constraint> = self.columns.iter().map(|c| c.width).collect();

        let block_title = if self.filter.is_empty() {
            format!(" {} ({}) ", title, self.filtered_indices.len())
        } else {
            format!(
                " {} ({}/{}) filter: {} ",
                title,
                self.filtered_indices.len(),
                self.all_rows.len(),
                self.filter
            )
        };

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(block_title))
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Constraint;

    fn sample_table() -> TableWidget {
        let cols = vec![
            Column {
                header: "NAME",
                width: Constraint::Min(10),
            },
            Column {
                header: "STATUS",
                width: Constraint::Length(10),
            },
        ];
        TableWidget::new(cols)
    }

    fn rows(data: &[(&str, &str)]) -> Vec<TableRow> {
        data.iter()
            .map(|(n, s)| TableRow::new(vec![n.to_string(), s.to_string()]))
            .collect()
    }

    #[test]
    fn set_rows_updates_count() {
        let mut t = sample_table();
        t.set_rows(rows(&[("pod-a", "Running"), ("pod-b", "Pending")]));
        assert_eq!(t.row_count(), 2);
    }

    #[test]
    fn filter_reduces_visible_rows() {
        let mut t = sample_table();
        t.set_rows(rows(&[
            ("pod-a", "Running"),
            ("pod-b", "Pending"),
            ("job-1", "Running"),
        ]));
        t.set_filter("pod".to_string());
        assert_eq!(t.row_count(), 2);
    }

    #[test]
    fn clear_filter_restores_all_rows() {
        let mut t = sample_table();
        t.set_rows(rows(&[("pod-a", "Running"), ("pod-b", "Pending")]));
        t.set_filter("pod-a".to_string());
        assert_eq!(t.row_count(), 1);
        t.set_filter(String::new());
        assert_eq!(t.row_count(), 2);
    }

    #[test]
    fn down_wraps_to_top() {
        let mut t = sample_table();
        t.set_rows(rows(&[("a", "1"), ("b", "2")]));
        t.bottom();
        t.down();
        assert_eq!(t.state.selected(), Some(0));
    }

    #[test]
    fn up_wraps_to_bottom() {
        let mut t = sample_table();
        t.set_rows(rows(&[("a", "1"), ("b", "2")]));
        t.top();
        t.up();
        assert_eq!(t.state.selected(), Some(1));
    }

    #[test]
    fn sort_by_column_toggles_direction() {
        let mut t = sample_table();
        t.set_rows(rows(&[("b", "x"), ("a", "y")]));
        t.sort_by_column(0);
        assert_eq!(t.sort_dir, SortDir::Ascending);
        t.sort_by_column(0);
        assert_eq!(t.sort_dir, SortDir::Descending);
    }
}
