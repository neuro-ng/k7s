//! XRay view — Phase 6.19.
//!
//! Displays Kubernetes resources as a hierarchical tree, making ownership
//! and dependency relationships immediately visible.  Opened with `:xray`.
//!
//! # Tree Structure
//!
//! ```text
//! Namespace: default
//! ├── Deployment: nginx  [3/3 ready]
//! │   └── ReplicaSet: nginx-6b7d4c (3/3)
//! │       ├── Pod: nginx-6b7d4c-abc12  Running  ✓
//! │       ├── Pod: nginx-6b7d4c-def34  Running  ✓
//! │       └── Pod: nginx-6b7d4c-ghi56  Pending  ⚠
//! └── Service: nginx-svc  ClusterIP
//! ```
//!
//! # Key Bindings
//!
//! | Key | Action |
//! |-----|--------|
//! | `↑` / `k` | Move cursor up |
//! | `↓` / `j` | Move cursor down |
//! | `Enter` / `Space` | Expand / collapse node |
//! | `e` | Expand all |
//! | `c` | Collapse all |
//! | `Esc` / `q` | Close XRay view |
//!
//! # k9s Reference
//! `internal/view/xray.go`

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

// ─── Node status ─────────────────────────────────────────────────────────────

/// Health status of a tree node.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NodeStatus {
    /// Resource is healthy / running.
    Ok,
    /// Resource has a non-fatal warning condition.
    Warning,
    /// Resource is in an error state.
    Error,
    /// Status is not yet known (e.g. still connecting).
    #[default]
    Unknown,
}

impl NodeStatus {
    /// A short indicator glyph for inline display.
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warning => "⚠",
            Self::Error => "✗",
            Self::Unknown => "?",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Ok => Color::Green,
            Self::Warning => Color::Yellow,
            Self::Error => Color::Red,
            Self::Unknown => Color::DarkGray,
        }
    }
}

// ─── XRay node ────────────────────────────────────────────────────────────────

/// A single node in the XRay resource tree.
#[derive(Debug, Clone)]
pub struct XRayNode {
    /// Kubernetes kind, e.g. `"Deployment"`, `"Pod"`.
    pub kind: String,
    /// Resource name.
    pub name: String,
    /// Namespace (if namespaced).
    pub namespace: Option<String>,
    /// Extra detail shown inline (e.g. `"3/3"`, `"Running"`, `"ClusterIP"`).
    pub detail: String,
    /// Health status indicator.
    pub status: NodeStatus,
    /// Child nodes (owned resources, endpoints, containers, etc.).
    pub children: Vec<XRayNode>,
    /// Whether this node's children are currently visible.
    expanded: bool,
}

impl XRayNode {
    pub fn new(
        kind: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
        status: NodeStatus,
    ) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            namespace: None,
            detail: detail.into(),
            status,
            children: Vec::new(),
            expanded: true,
        }
    }

    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn with_child(mut self, child: XRayNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    pub fn toggle_expand(&mut self) {
        if !self.children.is_empty() {
            self.expanded = !self.expanded;
        }
    }

    pub fn expand_all(&mut self) {
        self.expanded = true;
        for c in &mut self.children {
            c.expand_all();
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded = false;
        for c in &mut self.children {
            c.collapse_all();
        }
    }
}

// ─── Flattened render item ────────────────────────────────────────────────────

/// A flattened reference to a node for rendering.
struct FlatItem<'a> {
    node: &'a XRayNode,
    depth: usize,
    /// Is this the last sibling in its parent's children list?
    is_last: bool,
    /// Prefix built from ancestors' last-sibling state.
    prefix_tail: Vec<bool>,
}

/// Flatten the visible portion of the tree into a list of [`FlatItem`]s,
/// traversing only expanded branches.
fn flatten<'a>(roots: &'a [XRayNode]) -> Vec<FlatItem<'a>> {
    let mut out = Vec::new();
    flatten_nodes(roots, 0, &[], &mut out);
    out
}

fn flatten_nodes<'a>(
    nodes: &'a [XRayNode],
    depth: usize,
    prefix_tail: &[bool],
    out: &mut Vec<FlatItem<'a>>,
) {
    let n = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == n - 1;
        out.push(FlatItem {
            node,
            depth,
            is_last,
            prefix_tail: prefix_tail.to_vec(),
        });

        if node.expanded && !node.children.is_empty() {
            let mut new_prefix = prefix_tail.to_vec();
            new_prefix.push(is_last);
            flatten_nodes(&node.children, depth + 1, &new_prefix, out);
        }
    }
}

// ─── XRay view ────────────────────────────────────────────────────────────────

/// Action returned by [`XRayView::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XRayAction {
    /// User pressed `q` / `Esc` — close the view.
    Close,
    /// Key consumed, no structural change.
    None,
}

/// The XRay resource-tree view.
pub struct XRayView {
    /// Root nodes (typically one per namespace).
    roots: Vec<XRayNode>,
    /// Index into the currently-visible flattened list.
    cursor: usize,
    /// Scroll offset (first visible row index in the flat list).
    scroll: usize,
}

impl XRayView {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            cursor: 0,
            scroll: 0,
        }
    }

    /// Replace the tree contents.  The cursor is reset to the top.
    pub fn set_roots(&mut self, roots: Vec<XRayNode>) {
        self.roots = roots;
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Return the number of currently-visible flat items.
    pub fn visible_count(&self) -> usize {
        flatten(&self.roots).len()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> XRayAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return XRayAction::Close,

            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.visible_count().saturating_sub(1);
                if self.cursor < max {
                    self.cursor += 1;
                    self.ensure_cursor_visible();
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_cursor_node();
            }
            KeyCode::Char('e') => {
                for r in &mut self.roots {
                    r.expand_all();
                }
            }
            KeyCode::Char('c') => {
                for r in &mut self.roots {
                    r.collapse_all();
                }
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.visible_count().saturating_sub(1);
                self.ensure_cursor_visible();
            }
            _ => {}
        }
        XRayAction::None
    }

    /// Render the XRay view into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" XRay — Resource Tree ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let flat = flatten(&self.roots);
        let height = inner.height as usize;

        let lines: Vec<Line> = flat
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(height)
            .map(|(idx, item)| self.render_item(item, idx == self.cursor))
            .collect();

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);

        // Footer hints.
        let hints = " ↑↓ Move  Enter Toggle  e Expand All  c Collapse All  q Close ";
        let hint_area = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(1),
            width: area.width.saturating_sub(2).min(hints.len() as u16),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(hints).style(Style::default().fg(Color::DarkGray)),
            hint_area,
        );
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    fn render_item<'a>(&self, item: &FlatItem<'a>, selected: bool) -> Line<'a> {
        let mut spans: Vec<Span> = Vec::new();

        // Build the tree-drawing prefix from ancestor last-sibling flags.
        for &ancestor_is_last in &item.prefix_tail {
            spans.push(Span::styled(
                if ancestor_is_last { "   " } else { "│  " },
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Branch connector for this node.
        if item.depth > 0 {
            spans.push(Span::styled(
                if item.is_last { "└── " } else { "├── " },
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Expand/collapse indicator.
        if !item.node.is_leaf() {
            spans.push(Span::styled(
                if item.node.expanded { "▼ " } else { "▶ " },
                Style::default().fg(Color::Yellow),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // Kind label.
        let kind_color = kind_color(&item.node.kind);
        spans.push(Span::styled(
            format!("{}: ", item.node.kind),
            Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
        ));

        // Name.
        let name_style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(item.node.name.clone(), name_style));

        // Detail (e.g. "3/3", "Running").
        if !item.node.detail.is_empty() {
            spans.push(Span::styled(
                format!("  {}", item.node.detail),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Status glyph.
        spans.push(Span::styled(
            format!("  {}", item.node.status.glyph()),
            Style::default().fg(item.node.status.color()),
        ));

        Line::from(spans)
    }

    /// Toggle expand/collapse on the node at `cursor`.
    fn toggle_cursor_node(&mut self) {
        let flat = flatten(&self.roots);
        if let Some(item) = flat.get(self.cursor) {
            // We have a reference to the data but need a mutable path to the
            // actual node.  Re-traverse the tree using the depth + path.
            // Simplest correct approach: walk by matching kind/name at depth.
            let kind = item.node.kind.clone();
            let name = item.node.name.clone();
            let depth = item.depth;
            toggle_node_in(&mut self.roots, &kind, &name, depth, 0);
        }
    }

    /// Scroll so the cursor is always in the visible window.
    fn ensure_cursor_visible(&mut self) {
        // Use a fixed viewport height estimate (real height is only known at render time).
        let viewport = 40usize;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + viewport {
            self.scroll = self.cursor - viewport + 1;
        }
    }
}

impl Default for XRayView {
    fn default() -> Self {
        Self::new()
    }
}

/// Return a distinguishing colour for a resource kind.
fn kind_color(kind: &str) -> Color {
    match kind {
        "Namespace" => Color::Blue,
        "Deployment" | "StatefulSet" | "DaemonSet" => Color::Cyan,
        "ReplicaSet" => Color::LightCyan,
        "Pod" => Color::Green,
        "Container" => Color::LightGreen,
        "Service" => Color::Magenta,
        "ConfigMap" => Color::Yellow,
        "Secret" => Color::Red,
        "Node" => Color::White,
        _ => Color::Gray,
    }
}

/// Recursively find and toggle the node matching (`kind`, `name`) at `target_depth`.
fn toggle_node_in(
    nodes: &mut [XRayNode],
    kind: &str,
    name: &str,
    target_depth: usize,
    current_depth: usize,
) -> bool {
    for node in nodes.iter_mut() {
        if current_depth == target_depth && node.kind == kind && node.name == name {
            node.toggle_expand();
            return true;
        }
        if node.expanded
            && current_depth < target_depth
            && toggle_node_in(&mut node.children, kind, name, target_depth, current_depth + 1)
        {
            return true;
        }
    }
    false
}

// ─── Demo tree builder ────────────────────────────────────────────────────────

/// Build a static demonstration tree.
///
/// Used when the real watcher factory is not yet connected.  The caller
/// replaces this with live-data nodes once the cluster is available.
pub fn demo_tree() -> Vec<XRayNode> {
    vec![
        XRayNode::new("Namespace", "default", "", NodeStatus::Ok)
            .with_child(
                XRayNode::new("Deployment", "nginx", "3/3", NodeStatus::Ok)
                    .with_child(
                        XRayNode::new("ReplicaSet", "nginx-6b7d4c9bdf", "3/3", NodeStatus::Ok)
                            .with_child(
                                XRayNode::new("Pod", "nginx-6b7d4c9bdf-xk7p2", "Running", NodeStatus::Ok)
                                    .with_child(XRayNode::new("Container", "nginx", "1.27.0", NodeStatus::Ok))
                            )
                            .with_child(
                                XRayNode::new("Pod", "nginx-6b7d4c9bdf-m9r3t", "Running", NodeStatus::Ok)
                                    .with_child(XRayNode::new("Container", "nginx", "1.27.0", NodeStatus::Ok))
                            )
                            .with_child(
                                XRayNode::new("Pod", "nginx-6b7d4c9bdf-w2qs8", "Pending", NodeStatus::Warning)
                                    .with_child(XRayNode::new("Container", "nginx", "waiting", NodeStatus::Warning))
                            ),
                    ),
            )
            .with_child(XRayNode::new("Service", "nginx-svc", "ClusterIP:10.96.0.1", NodeStatus::Ok))
            .with_child(
                XRayNode::new("Deployment", "api-server", "1/2", NodeStatus::Warning)
                    .with_child(
                        XRayNode::new("ReplicaSet", "api-server-7c9fbd5f", "1/2", NodeStatus::Warning)
                            .with_child(
                                XRayNode::new("Pod", "api-server-7c9fbd5f-abcd1", "Running", NodeStatus::Ok)
                                    .with_child(XRayNode::new("Container", "api", "v2.3.1", NodeStatus::Ok))
                            )
                            .with_child(
                                XRayNode::new("Pod", "api-server-7c9fbd5f-efgh2", "CrashLoopBackOff", NodeStatus::Error)
                                    .with_child(XRayNode::new("Container", "api", "OOMKilled", NodeStatus::Error))
                            ),
                    ),
            ),
        XRayNode::new("Namespace", "kube-system", "", NodeStatus::Ok)
            .with_child(
                XRayNode::new("Deployment", "coredns", "2/2", NodeStatus::Ok)
                    .with_child(
                        XRayNode::new("ReplicaSet", "coredns-787d4945fb", "2/2", NodeStatus::Ok)
                            .with_child(XRayNode::new("Pod", "coredns-787d4945fb-abc12", "Running", NodeStatus::Ok))
                            .with_child(XRayNode::new("Pod", "coredns-787d4945fb-def34", "Running", NodeStatus::Ok)),
                    ),
            )
            .with_child(XRayNode::new("Node", "node-1", "Ready", NodeStatus::Ok))
            .with_child(XRayNode::new("Node", "node-2", "Ready", NodeStatus::Ok)),
    ]
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn simple_tree() -> Vec<XRayNode> {
        vec![
            XRayNode::new("Namespace", "default", "", NodeStatus::Ok).with_child(
                XRayNode::new("Deployment", "nginx", "3/3", NodeStatus::Ok)
                    .with_child(XRayNode::new("Pod", "pod-a", "Running", NodeStatus::Ok))
                    .with_child(XRayNode::new("Pod", "pod-b", "Running", NodeStatus::Ok)),
            ),
        ]
    }

    #[test]
    fn flatten_all_visible_when_expanded() {
        let roots = simple_tree();
        let flat = flatten(&roots);
        // Namespace → Deployment → Pod-a → Pod-b = 4 nodes
        assert_eq!(flat.len(), 4);
    }

    #[test]
    fn flatten_hides_collapsed_children() {
        let mut roots = simple_tree();
        // Collapse the Deployment node.
        roots[0].children[0].expanded = false;
        let flat = flatten(&roots);
        // Only Namespace + Deployment visible (pods hidden)
        assert_eq!(flat.len(), 2);
    }

    #[test]
    fn cursor_moves_down_and_wraps_at_end() {
        let mut view = XRayView::new();
        view.set_roots(simple_tree());
        assert_eq!(view.cursor, 0);
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.cursor, 1);
        view.handle_key(&key(KeyCode::Down));
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.cursor, 3);
        // At the last item — should not move further.
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.cursor, 3);
    }

    #[test]
    fn cursor_moves_up_and_stops_at_zero() {
        let mut view = XRayView::new();
        view.set_roots(simple_tree());
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.cursor, 1);
        view.handle_key(&key(KeyCode::Up));
        assert_eq!(view.cursor, 0);
        // Already at top — should stay at 0.
        view.handle_key(&key(KeyCode::Up));
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn toggle_collapses_and_expands_deployment() {
        let mut view = XRayView::new();
        view.set_roots(simple_tree());
        // Cursor is at 0 (Namespace). Move to Deployment (index 1).
        view.handle_key(&key(KeyCode::Down));
        assert_eq!(view.cursor, 1);
        // Toggle: should collapse the Deployment's children.
        view.handle_key(&key(KeyCode::Enter));
        let visible = view.visible_count();
        // Namespace + Deployment only (pods hidden)
        assert_eq!(visible, 2);
        // Toggle again: re-expand.
        view.handle_key(&key(KeyCode::Enter));
        assert_eq!(view.visible_count(), 4);
    }

    #[test]
    fn collapse_all_leaves_only_roots() {
        let mut view = XRayView::new();
        view.set_roots(simple_tree());
        view.handle_key(&key(KeyCode::Char('c')));
        // Only root node visible (Namespace is collapsed, showing only itself).
        assert_eq!(view.visible_count(), 1);
    }

    #[test]
    fn expand_all_shows_full_tree() {
        let mut view = XRayView::new();
        let mut roots = simple_tree();
        roots[0].expanded = false;
        view.set_roots(roots);
        view.handle_key(&key(KeyCode::Char('e')));
        assert_eq!(view.visible_count(), 4);
    }

    #[test]
    fn q_returns_close_action() {
        let mut view = XRayView::new();
        view.set_roots(simple_tree());
        let action = view.handle_key(&key(KeyCode::Char('q')));
        assert_eq!(action, XRayAction::Close);
    }

    #[test]
    fn node_status_glyphs_are_distinct() {
        assert_ne!(NodeStatus::Ok.glyph(), NodeStatus::Error.glyph());
        assert_ne!(NodeStatus::Warning.glyph(), NodeStatus::Unknown.glyph());
    }

    #[test]
    fn demo_tree_has_two_namespaces() {
        let tree = demo_tree();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].kind, "Namespace");
        assert_eq!(tree[1].kind, "Namespace");
    }

    #[test]
    fn leaf_node_is_leaf() {
        let n = XRayNode::new("Container", "app", "v1", NodeStatus::Ok);
        assert!(n.is_leaf());
    }

    #[test]
    fn non_leaf_node_is_not_leaf() {
        let n = XRayNode::new("Deployment", "nginx", "1/1", NodeStatus::Ok)
            .with_child(XRayNode::new("Pod", "pod-a", "Running", NodeStatus::Ok));
        assert!(!n.is_leaf());
    }
}
