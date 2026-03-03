pub mod chart;
pub mod footer;
pub mod header;
pub mod table;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, TableState, Wrap},
    Frame,
};

use crate::config::{ColumnVisibility, SortConfig};
use crate::model::{VagrantEnvironment, ViewMode};
use crate::vagrant::ConnectionState;

/// Render the entire UI.
pub fn render(
    frame: &mut Frame,
    environments: &[VagrantEnvironment],
    table_state: &mut TableState,
    view: ViewMode,
    columns: &ColumnVisibility,
    sort: &SortConfig,
    connection: ConnectionState,
    last_poll_secs: u64,
    show_help: bool,
    hide_footer: bool,
    error_msg: &Option<String>,
) {
    let constraints = if hide_footer {
        vec![
            Constraint::Length(1), // header
            Constraint::Min(5),   // main content
        ]
    } else {
        vec![
            Constraint::Length(1), // header
            Constraint::Min(5),   // main content
            Constraint::Length(1), // footer
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    header::render(frame, chunks[0], connection, last_poll_secs);

    match view {
        ViewMode::Table => {
            table::render(frame, chunks[1], environments, table_state, columns, sort);
        }
        ViewMode::Chart => {
            chart::render(frame, chunks[1], environments);
        }
    }

    if !hide_footer {
        footer::render(frame, chunks[2], show_help);
    }

    // Error overlay
    if let Some(msg) = error_msg {
        render_error_overlay(frame, msg);
    }

    // Help overlay
    if show_help {
        render_help_overlay(frame);
    }
}

fn render_error_overlay(frame: &mut Frame, msg: &str) {
    let area = centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Error ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DANGER));

    let text = Paragraph::new(msg.to_string())
        .block(block)
        .wrap(Wrap { trim: true });

    frame.render_widget(text, area);
}

fn render_help_overlay(frame: &mut Frame) {
    let area = centered_rect(50, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::PRIMARY));

    let help_lines = vec![
        help_line("q / Ctrl+C", "Quit"),
        help_line("j / Down", "Next row"),
        help_line("k / Up", "Previous row"),
        help_line("g", "Jump to top"),
        help_line("G", "Jump to bottom"),
        help_line("Tab", "Toggle Table / Chart view"),
        help_line("1-7", "Toggle columns"),
        help_line("s", "Cycle sort column forward"),
        help_line("S", "Cycle sort column backward"),
        help_line("r", "Reverse sort direction"),
        help_line("?", "Toggle this help"),
    ];

    let text = Paragraph::new(help_lines).block(block);
    frame.render_widget(text, area);
}

fn help_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {:>12}  ", key),
            Style::default()
                .fg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, Style::default().fg(theme::MUTED)),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
