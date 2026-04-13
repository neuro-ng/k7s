//! Splash / welcome screen — Phase 5.9.
//!
//! Rendered when no browser view is active (no cluster connected, or first launch).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// ASCII-art logo lines for "k7s".
const LOGO: &[&str] = &[
    r" _    ______  ",
    r"| | _|___  /___",
    r"| |/ /  / // __|",
    r"|   <  / / \__ \",
    r"|_|\_\/_/  |___/",
];

/// Render the splash screen into `area`.
///
/// Shows the k7s logo, version, tagline, and key-binding quick-start.
pub fn render_splash(frame: &mut Frame, area: Rect, version: &str) {
    // Vertical layout: top padding / logo / spacer / hints / bottom padding.
    let logo_height = LOGO.len() as u16;
    let hints_height = 7u16;
    let total_content = logo_height + 1 + hints_height;

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(area.height.saturating_sub(total_content) / 2),
            Constraint::Length(logo_height),
            Constraint::Length(1),
            Constraint::Length(hints_height),
            Constraint::Min(0),
        ])
        .split(area);

    // ── Logo ──────────────────────────────────────────────────────────────────
    let logo_lines: Vec<Line> = LOGO
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                *l,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(logo_lines).alignment(Alignment::Center),
        vert[1],
    );

    // ── Version tag line ──────────────────────────────────────────────────────
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" v{version} ", version = version),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                " Security-First Kubernetes TUI + AI ",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .alignment(Alignment::Center),
        vert[2],
    );

    // ── Quick-start hints ─────────────────────────────────────────────────────
    let hint_style = Style::default().fg(Color::DarkGray);
    let key_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    let hints: Vec<Line> = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(":pods  ", key_style),
            Span::styled("browse pods", hint_style),
            Span::raw("    "),
            Span::styled(":nodes  ", key_style),
            Span::styled("browse nodes", hint_style),
            Span::raw("    "),
            Span::styled(":deploy  ", key_style),
            Span::styled("browse deployments", hint_style),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(":alias  ", key_style),
            Span::styled("all resource aliases", hint_style),
            Span::raw("    "),
            Span::styled(":pulse  ", key_style),
            Span::styled("cluster dashboard", hint_style),
            Span::raw("    "),
            Span::styled(":ctx  ", key_style),
            Span::styled("switch context", hint_style),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Space  ", key_style),
            Span::styled("open AI chat", hint_style),
            Span::raw("    "),
            Span::styled("?  ", key_style),
            Span::styled("help", hint_style),
            Span::raw("    "),
            Span::styled("q  ", key_style),
            Span::styled("quit", hint_style),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  No cluster connected — configure kubeconfig and restart.",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )),
    ];

    frame.render_widget(Paragraph::new(hints).alignment(Alignment::Left), vert[3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_is_non_empty() {
        assert!(!LOGO.is_empty());
        assert!(LOGO.iter().all(|l| !l.is_empty()));
    }

    #[test]
    fn logo_lines_same_width() {
        // All lines should be padded to similar length (within 2 chars).
        let widths: Vec<usize> = LOGO.iter().map(|l| l.len()).collect();
        let min = *widths.iter().min().unwrap();
        let max = *widths.iter().max().unwrap();
        assert!(max - min <= 4, "logo lines differ too much: {min} vs {max}");
    }
}
