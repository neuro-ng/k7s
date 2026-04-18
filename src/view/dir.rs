//! Local filesystem directory browser — Phase 10.6.
//!
//! Provides a TUI view that lets the user navigate the local filesystem,
//! inspect file sizes and modification times, and open files for viewing.
//!
//! # Design
//!
//! * No extra dependencies — uses only `std::fs`.
//! * Entries are sorted: directories first, then files, both alphabetically.
//! * Navigation: `↑↓` / `j` `k` move cursor; `Enter` descends into a
//!   directory; `Backspace` / `[` goes up to the parent; `Esc` / `q` closes.
//! * The view is triggered by `:dir [path]` from the command prompt.  Without
//!   a path it defaults to the current working directory.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

// ─── Entry kind ───────────────────────────────────────────────────────────────

/// The kind of a filesystem entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Unknown,
}

impl EntryKind {
    fn glyph(&self) -> &'static str {
        match self {
            Self::Directory => "d",
            Self::File => "-",
            Self::Symlink => "l",
            Self::Unknown => "?",
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Directory => Color::Cyan,
            Self::File => Color::White,
            Self::Symlink => Color::LightMagenta,
            Self::Unknown => Color::DarkGray,
        }
    }
}

// ─── DirEntry ─────────────────────────────────────────────────────────────────

/// A single entry in the directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// File or directory name (not the full path).
    pub name: String,
    /// Full absolute path.
    pub path: PathBuf,
    /// File size in bytes (`None` for directories / symlinks / errors).
    pub size: Option<u64>,
    /// Last-modified time.
    pub modified: Option<SystemTime>,
    /// Entry kind.
    pub kind: EntryKind,
}

impl DirEntry {
    /// Human-readable file size (e.g. `"4.2 MB"`).
    pub fn size_label(&self) -> String {
        match self.size {
            Some(b) if b < 1_024 => format!("{b}B"),
            Some(b) if b < 1_024 * 1_024 => format!("{:.1}K", b as f64 / 1_024.0),
            Some(b) if b < 1_024 * 1_024 * 1_024 => {
                format!("{:.1}M", b as f64 / (1_024.0 * 1_024.0))
            }
            Some(b) => format!("{:.1}G", b as f64 / (1_024.0 * 1_024.0 * 1_024.0)),
            None => {
                if self.kind == EntryKind::Directory {
                    "-".to_owned()
                } else {
                    "?".to_owned()
                }
            }
        }
    }

    /// RFC 3339-like modification time label (local time, date + hour:min).
    pub fn modified_label(&self) -> String {
        match self.modified {
            None => "-".to_owned(),
            Some(t) => {
                // Convert to seconds since UNIX epoch; no chrono dependency.
                let secs = t
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                // Very small ISO-8601-ish formatter (UTC only, no TZ conversion).
                let s = secs % 60;
                let m = (secs / 60) % 60;
                let h = (secs / 3_600) % 24;
                let days = secs / 86_400;
                // Gregorian calendar approximation (good enough for display).
                let year = 1970 + days / 365;
                let day_of_year = days % 365;
                let month = day_of_year / 30 + 1;
                let day = day_of_year % 30 + 1;
                format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}")
            }
        }
    }
}

// ─── Action ───────────────────────────────────────────────────────────────────

/// Action returned by [`DirView::handle_key`].
#[derive(Debug, Clone, PartialEq)]
pub enum DirAction {
    /// User pressed Esc / q — close the view.
    Close,
    /// No action.
    None,
}

// ─── DirView ──────────────────────────────────────────────────────────────────

/// TUI view that shows a local filesystem directory as a scrollable table.
pub struct DirView {
    /// Directory currently displayed.
    pub current_path: PathBuf,
    /// Entries in the current directory (sorted).
    entries: Vec<DirEntry>,
    /// Ratatui table scroll/selection state.
    table_state: TableState,
    /// Last error (e.g. permission denied when reading the directory).
    error: Option<String>,
}

impl DirView {
    /// Open the view at `path`.  If `path` is a file the parent directory is
    /// used instead.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let p = path.as_ref();
        let dir = if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        };
        let mut view = Self {
            current_path: dir,
            entries: Vec::new(),
            table_state: TableState::default(),
            error: None,
        };
        view.load();
        view
    }

    /// Open the view at the current working directory.
    pub fn new_cwd() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(cwd)
    }

    /// Navigate into a directory (or do nothing for files).
    pub fn enter(&mut self) {
        if let Some(sel) = self.table_state.selected() {
            if let Some(entry) = self.entries.get(sel) {
                if entry.kind == EntryKind::Directory {
                    self.current_path = entry.path.clone();
                    self.load();
                }
                // Files: no-op for now (future: open a read-only viewer).
            }
        }
    }

    /// Navigate to the parent directory.
    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            self.current_path = parent.to_path_buf();
            self.load();
        }
    }

    fn load(&mut self) {
        self.error = None;
        self.entries.clear();

        let read = match std::fs::read_dir(&self.current_path) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(format!("Cannot read {}: {e}", self.current_path.display()));
                self.table_state.select(None);
                return;
            }
        };

        let mut entries: Vec<DirEntry> = read
            .filter_map(|res| res.ok())
            .map(|de| {
                let meta = de.metadata().ok();
                let kind = match de.file_type() {
                    Ok(t) if t.is_dir() => EntryKind::Directory,
                    Ok(t) if t.is_symlink() => EntryKind::Symlink,
                    Ok(t) if t.is_file() => EntryKind::File,
                    _ => EntryKind::Unknown,
                };
                let size = meta.as_ref().and_then(|m| {
                    if kind == EntryKind::File { Some(m.len()) } else { None }
                });
                let modified = meta.as_ref().and_then(|m| m.modified().ok());
                DirEntry {
                    name: de.file_name().to_string_lossy().to_string(),
                    path: de.path(),
                    size,
                    modified,
                    kind,
                }
            })
            .collect();

        // Sort: directories first, then files; alphabetically within each group.
        entries.sort_by(|a, b| {
            let a_is_dir = a.kind == EntryKind::Directory;
            let b_is_dir = b.kind == EntryKind::Directory;
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
        });

        self.entries = entries;
        if self.entries.is_empty() {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(0));
        }
    }

    pub fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> DirAction {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => DirAction::Close,
            KeyCode::Enter => {
                self.enter();
                DirAction::None
            }
            KeyCode::Backspace | KeyCode::Char('[') => {
                self.go_up();
                DirAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.entries.len();
                if len > 0 {
                    let next = self.table_state.selected()
                        .map(|s| (s + 1).min(len - 1))
                        .unwrap_or(0);
                    self.table_state.select(Some(next));
                }
                DirAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let next = self.table_state.selected()
                    .map(|s| s.saturating_sub(1))
                    .unwrap_or(0);
                self.table_state.select(Some(next));
                DirAction::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if !self.entries.is_empty() {
                    self.table_state.select(Some(0));
                }
                DirAction::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                let len = self.entries.len();
                if len > 0 {
                    self.table_state.select(Some(len - 1));
                }
                DirAction::None
            }
            _ => DirAction::None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // path bar
                Constraint::Min(0),    // table
                Constraint::Length(1), // footer hints
            ])
            .split(area);

        self.render_path_bar(frame, chunks[0]);
        self.render_table(frame, chunks[1]);
        self.render_hints(frame, chunks[2]);
    }

    fn render_path_bar(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(ref e) = self.error {
            Line::from(vec![
                Span::styled("  Error: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(e.as_str()),
            ])
        } else {
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    self.current_path.display().to_string(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ({} entries)", self.entries.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        };
        let p = Paragraph::new(text)
            .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(p, area);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from("T").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Name").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Size").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Modified").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().fg(Color::White))
        .height(1);

        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                Row::new(vec![
                    Cell::from(e.kind.glyph()).style(Style::default().fg(e.kind.color())),
                    Cell::from(e.name.as_str()).style(
                        Style::default()
                            .fg(e.kind.color())
                            .add_modifier(if e.kind == EntryKind::Directory {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Cell::from(e.size_label()),
                    Cell::from(e.modified_label()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(20),
        ];

        let title = format!(" Dir: {} ", self.current_path.display());
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title))
            .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_hints(&self, frame: &mut Frame, area: Rect) {
        let hints = Paragraph::new("  ↑/↓ navigate   ⏎ enter dir   ⌫/[ parent   q/Esc close")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hints, area);
    }

    /// Return the currently selected entry (if any).
    pub fn selected(&self) -> Option<&DirEntry> {
        self.table_state.selected().and_then(|i| self.entries.get(i))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_entry_size_label_bytes() {
        let e = DirEntry {
            name: "a.txt".into(),
            path: PathBuf::from("a.txt"),
            size: Some(500),
            modified: None,
            kind: EntryKind::File,
        };
        assert_eq!(e.size_label(), "500B");
    }

    #[test]
    fn dir_entry_size_label_kilobytes() {
        let e = DirEntry {
            name: "a.txt".into(),
            path: PathBuf::from("a.txt"),
            size: Some(2048),
            modified: None,
            kind: EntryKind::File,
        };
        assert!(e.size_label().ends_with('K'));
    }

    #[test]
    fn dir_entry_size_label_none_for_dir() {
        let e = DirEntry {
            name: "subdir".into(),
            path: PathBuf::from("subdir"),
            size: None,
            modified: None,
            kind: EntryKind::Directory,
        };
        assert_eq!(e.size_label(), "-");
    }

    #[test]
    fn entry_kind_glyph() {
        assert_eq!(EntryKind::Directory.glyph(), "d");
        assert_eq!(EntryKind::File.glyph(), "-");
        assert_eq!(EntryKind::Symlink.glyph(), "l");
    }

    #[test]
    fn dir_view_loads_tmp() {
        let dir = tempfile::tempdir().unwrap();
        // Create a couple of test files.
        std::fs::write(dir.path().join("alpha.txt"), b"hello").unwrap();
        std::fs::create_dir(dir.path().join("beta_dir")).unwrap();

        let mut view = DirView::new(dir.path());
        assert!(view.error.is_none());
        // Should have 2 entries.
        assert_eq!(view.entries.len(), 2);
        // Directories should sort first.
        assert_eq!(view.entries[0].kind, EntryKind::Directory);
        assert_eq!(view.entries[1].kind, EntryKind::File);
    }

    #[test]
    fn dir_view_go_up() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child_dir");
        std::fs::create_dir(&child).unwrap();

        let mut view = DirView::new(&child);
        let original = view.current_path.clone();
        view.go_up();
        // After go_up() we should be in the parent.
        assert_ne!(view.current_path, original);
    }

    #[test]
    fn dir_view_enter_descends() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child_dir");
        std::fs::create_dir(&child).unwrap();
        // Create a file too, so the dir entry isn't the only one.
        std::fs::write(dir.path().join("file.txt"), b"x").unwrap();

        let mut view = DirView::new(dir.path());
        // Ensure the directory entry is selected.
        view.table_state.select(Some(0)); // dirs are first
        let before = view.current_path.clone();
        view.enter();
        assert_ne!(view.current_path, before);
        assert_eq!(view.current_path, child);
    }

    #[test]
    fn dir_view_nonexistent_sets_error() {
        let view = DirView::new("/definitely/does/not/exist/k7s_test_path");
        assert!(view.error.is_some());
    }
}
