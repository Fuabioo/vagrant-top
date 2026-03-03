use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::theme::{MUTED, PRIMARY};

pub fn render(frame: &mut Frame, area: Rect, show_help: bool) {
    let bindings: &[(&str, &str)] = if show_help {
        &[("q", "quit"), ("?", "close help")]
    } else {
        &[
            ("q", "quit"),
            ("j/k", "nav"),
            ("Tab", "view"),
            ("1-7", "columns"),
            ("s", "sort"),
            ("r", "reverse"),
            ("?", "help"),
        ]
    };

    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, desc)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default().fg(MUTED)));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {}", desc),
            Style::default().fg(MUTED),
        ));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
