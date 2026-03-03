use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::theme::{self, MUTED, PRIMARY, SUCCESS, WARNING};
use crate::vagrant::ConnectionState;

pub fn render(frame: &mut Frame, area: Rect, connection: ConnectionState, last_poll_secs: u64) {
    let icon = Span::styled(
        format!("{} ", theme::ICON_VAGRANT),
        ratatui::style::Style::default()
            .fg(PRIMARY)
            .add_modifier(Modifier::BOLD),
    );

    let title = Span::styled(
        "vagrant-status  ",
        ratatui::style::Style::default()
            .fg(PRIMARY)
            .add_modifier(Modifier::BOLD),
    );

    let status = match connection {
        ConnectionState::Connected => {
            Span::styled("Connected", ratatui::style::Style::default().fg(SUCCESS))
        }
        ConnectionState::IndexOnly => {
            Span::styled("Index Only", ratatui::style::Style::default().fg(WARNING))
        }
        ConnectionState::Disconnected => {
            Span::styled("Disconnected", ratatui::style::Style::default().fg(WARNING))
        }
    };

    let poll_text = format!("  Last poll: {}s ago", last_poll_secs);
    let poll = Span::styled(poll_text, ratatui::style::Style::default().fg(MUTED));

    let line = Line::from(vec![icon, title, status, poll]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
