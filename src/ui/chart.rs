use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{BarChart, Block, Borders},
    Frame,
};

use crate::model::VagrantEnvironment;
use super::theme;

pub fn render(frame: &mut Frame, area: Rect, environments: &[VagrantEnvironment]) {
    let direction = if area.width >= 100 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };

    let chunks = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_cpu_chart(frame, chunks[0], environments);
    render_mem_chart(frame, chunks[1], environments);
}

fn render_cpu_chart(frame: &mut Frame, area: Rect, environments: &[VagrantEnvironment]) {
    let data: Vec<(String, u64)> = environments
        .iter()
        .map(|e| (truncate_name(&e.name, 12), (e.total_cpu * 10.0) as u64))
        .collect();

    let bar_data: Vec<(&str, u64)> = data.iter().map(|(n, v)| (n.as_str(), *v)).collect();

    let chart = BarChart::default()
        .block(
            Block::default()
                .title(format!(" {} CPU % ", theme::ICON_CPU))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY)),
        )
        .data(&bar_data)
        .bar_width(5)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Cyan))
        .value_style(Style::default().fg(Color::White));

    frame.render_widget(chart, area);
}

fn render_mem_chart(frame: &mut Frame, area: Rect, environments: &[VagrantEnvironment]) {
    let data: Vec<(String, u64)> = environments
        .iter()
        .map(|e| {
            let mib = e.total_mem / (1024 * 1024);
            (truncate_name(&e.name, 12), mib)
        })
        .collect();

    let bar_data: Vec<(&str, u64)> = data.iter().map(|(n, v)| (n.as_str(), *v)).collect();

    let chart = BarChart::default()
        .block(
            Block::default()
                .title(format!(" {} MEM (MiB) ", theme::ICON_MEMORY))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY)),
        )
        .data(&bar_data)
        .bar_width(5)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Magenta))
        .value_style(Style::default().fg(Color::White));

    frame.render_widget(chart, area);
}

fn truncate_name(name: &str, max_len: usize) -> String {
    if name.chars().count() <= max_len {
        name.to_string()
    } else {
        let truncated: String = name.chars().take(max_len - 1).collect();
        format!("{}…", truncated)
    }
}
