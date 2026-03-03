use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::config::{ColumnVisibility, SortColumn, SortConfig};
use crate::model::{self, VagrantEnvironment};

use super::theme;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    environments: &[VagrantEnvironment],
    table_state: &mut TableState,
    columns: &ColumnVisibility,
    sort: &SortConfig,
) {
    if environments.is_empty() {
        let msg = Paragraph::new("No Vagrant environments found")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme::MUTED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme::PRIMARY)),
            );
        frame.render_widget(msg, area);
        return;
    }

    let header_cells = build_header_cells(columns, sort);
    let header = Row::new(header_cells)
        .style(theme::style_header())
        .height(1);

    let rows: Vec<Row> = environments
        .iter()
        .map(|e| build_row(e, columns))
        .collect();

    let widths = build_widths(columns);

    let table = Table::new(rows, &widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme::PRIMARY)),
        )
        .row_highlight_style(theme::style_selected())
        .highlight_symbol("\u{25b8} ");

    frame.render_stateful_widget(table, area, table_state);
}

fn build_header_cells<'a>(columns: &ColumnVisibility, sort: &SortConfig) -> Vec<Cell<'a>> {
    let mut cells = vec![sort_header("ENVIRONMENT", SortColumn::Name, sort)];

    if columns.vms {
        cells.push(sort_header("VMS", SortColumn::Vms, sort));
    }
    if columns.cpu {
        cells.push(sort_header(
            &format!("{} CPU", theme::ICON_CPU),
            SortColumn::Cpu,
            sort,
        ));
    }
    if columns.mem {
        cells.push(sort_header(
            &format!("{} MEM", theme::ICON_MEMORY),
            SortColumn::Mem,
            sort,
        ));
    }
    if columns.net {
        cells.push(sort_header(
            &format!("{} NET RX/TX", theme::ICON_NETWORK),
            SortColumn::NetRx,
            sort,
        ));
    }
    if columns.io {
        cells.push(sort_header(
            &format!("{} IO R/W", theme::ICON_DISK),
            SortColumn::IoRead,
            sort,
        ));
    }
    if columns.time_up {
        cells.push(sort_header(
            &format!("{} TIME-UP", theme::ICON_CLOCK),
            SortColumn::TimeUp,
            sort,
        ));
    }
    if columns.last_chg {
        cells.push(sort_header(
            &format!("{} LAST-CHG", theme::ICON_CLOCK),
            SortColumn::LastChg,
            sort,
        ));
    }

    cells
}

fn sort_header<'a>(label: &str, col: SortColumn, sort: &SortConfig) -> Cell<'a> {
    if sort.column == col {
        let icon = if sort.ascending {
            theme::ICON_SORT_ASC
        } else {
            theme::ICON_SORT_DESC
        };
        Cell::from(format!("{} {}", label, icon))
    } else {
        Cell::from(label.to_string())
    }
}

fn build_row<'a>(env: &VagrantEnvironment, columns: &ColumnVisibility) -> Row<'a> {
    let icon = theme::status_icon(env.status);
    let icon_style = theme::style_status(env.status);

    let name_line = Line::from(vec![
        Span::styled(format!("{} ", icon), icon_style),
        Span::raw(env.name.clone()),
    ]);

    let mut cells: Vec<Cell> = vec![Cell::from(name_line)];

    if columns.vms {
        cells.push(Cell::from(format!("{}", env.vm_count())));
    }
    if columns.cpu {
        let cpu_str = format!("{:.1}%", env.total_cpu);
        cells.push(Cell::from(cpu_str).style(theme::style_cpu(env.total_cpu)));
    }
    if columns.mem {
        let mem_str = format!(
            "{} / {}",
            theme::fmt_bytes(env.total_mem),
            theme::fmt_bytes(env.mem_limit),
        );
        cells.push(Cell::from(mem_str).style(theme::style_mem(env.mem_percent())));
    }
    if columns.net {
        let net_str = format!(
            "{} / {}",
            theme::fmt_bytes(env.total_net_rx),
            theme::fmt_bytes(env.total_net_tx),
        );
        cells.push(Cell::from(net_str));
    }
    if columns.io {
        let io_str = format!(
            "{} / {}",
            theme::fmt_bytes(env.total_blk_read),
            theme::fmt_bytes(env.total_blk_write),
        );
        cells.push(Cell::from(io_str));
    }
    if columns.time_up {
        let time_str = env
            .oldest_started_at
            .map(|started| model::uptime_str(started.elapsed()))
            .unwrap_or_else(|| "-".to_string());
        cells.push(Cell::from(time_str));
    }
    if columns.last_chg {
        let chg_str = env
            .newest_started_at
            .map(|started| model::uptime_str(started.elapsed()))
            .unwrap_or_else(|| "-".to_string());
        cells.push(Cell::from(chg_str));
    }

    Row::new(cells)
}

fn build_widths(columns: &ColumnVisibility) -> Vec<Constraint> {
    let mut widths = vec![Constraint::Fill(1)]; // ENVIRONMENT gets all remaining space

    if columns.vms {
        widths.push(Constraint::Length(6));
    }
    if columns.cpu {
        widths.push(Constraint::Length(9));
    }
    if columns.mem {
        widths.push(Constraint::Length(17));
    }
    if columns.net {
        widths.push(Constraint::Length(21));
    }
    if columns.io {
        widths.push(Constraint::Length(15));
    }
    if columns.time_up {
        widths.push(Constraint::Length(10));
    }
    if columns.last_chg {
        widths.push(Constraint::Length(10));
    }

    widths
}
