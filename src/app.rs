use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::config::{ColumnVisibility, SortConfig};
use crate::event::AppEvent;
use crate::model::{VagrantEnvironment, ViewMode};
use crate::ui;
use crate::vagrant::ConnectionState;

/// Application state.
pub struct App {
    pub environments: Vec<VagrantEnvironment>,
    pub table_state: TableState,
    pub view: ViewMode,
    pub columns: ColumnVisibility,
    pub sort: SortConfig,
    pub connection: ConnectionState,
    pub show_help: bool,
    pub hide_footer: bool,
    pub error_msg: Option<String>,
    pub should_quit: bool,
    last_vagrant_update: Instant,
    last_rendered_poll_secs: u64,
    dirty: bool,
}

impl App {
    pub fn new(hide_footer: bool) -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        Self {
            environments: Vec::new(),
            table_state,
            view: ViewMode::Table,
            columns: ColumnVisibility::default(),
            sort: SortConfig::default(),
            connection: ConnectionState::Disconnected,
            show_help: false,
            hide_footer,
            error_msg: None,
            should_quit: false,
            last_vagrant_update: Instant::now(),
            last_rendered_poll_secs: 0,
            dirty: true,
        }
    }

    pub fn last_poll_secs(&self) -> u64 {
        self.last_vagrant_update.elapsed().as_secs()
    }

    pub fn selected(&self) -> usize {
        self.table_state.selected().unwrap_or(0)
    }

    /// Main event loop. Drains all pending events, renders on Tick if dirty.
    pub async fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        mut rx: mpsc::Receiver<AppEvent>,
    ) -> Result<()> {
        while !self.should_quit {
            let event = match rx.recv().await {
                Some(e) => e,
                None => break,
            };

            self.handle_event(event);

            // Drain any remaining pending events without blocking
            while let Ok(event) = rx.try_recv() {
                self.handle_event(event);
            }

            if self.dirty {
                self.dirty = false;
                self.draw(terminal)?;
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => {
                self.handle_key(key);
                self.dirty = true;
            }
            AppEvent::Resize(_, _) => {
                self.dirty = true;
            }
            AppEvent::Tick => {
                let cur = self.last_poll_secs();
                if cur != self.last_rendered_poll_secs {
                    self.last_rendered_poll_secs = cur;
                    self.dirty = true;
                }
            }
            AppEvent::VagrantUpdate(environments) => {
                self.environments = environments;
                self.apply_sort();
                self.connection = ConnectionState::Connected;
                self.error_msg = None;
                self.last_vagrant_update = Instant::now();
                self.last_rendered_poll_secs = 0;
                self.clamp_selection();
                self.dirty = true;
            }
            AppEvent::VagrantError {
                message,
                index_available,
            } => {
                self.error_msg = Some(message);
                self.connection = if index_available {
                    ConnectionState::IndexOnly
                } else {
                    ConnectionState::Disconnected
                };
                self.dirty = true;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),
            KeyCode::Tab => self.toggle_view(),
            KeyCode::Char('s') => {
                self.sort.column = self.sort.column.next();
                self.apply_sort();
            }
            KeyCode::Char('S') => {
                self.sort.column = self.sort.column.prev();
                self.apply_sort();
            }
            KeyCode::Char('r') => {
                self.sort.ascending = !self.sort.ascending;
                self.apply_sort();
            }
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char(c @ '1'..='7') => {
                let num = c as u8 - b'0';
                self.columns.toggle(num);
            }
            _ => {}
        }
    }

    fn select_next(&mut self) {
        if !self.environments.is_empty() {
            let i = self.selected();
            let next = (i + 1).min(self.environments.len() - 1);
            self.table_state.select(Some(next));
        }
    }

    fn select_prev(&mut self) {
        let i = self.selected();
        self.table_state.select(Some(i.saturating_sub(1)));
    }

    fn select_first(&mut self) {
        self.table_state.select(Some(0));
    }

    fn select_last(&mut self) {
        if !self.environments.is_empty() {
            self.table_state
                .select(Some(self.environments.len() - 1));
        }
    }

    fn toggle_view(&mut self) {
        self.view = match self.view {
            ViewMode::Table => ViewMode::Chart,
            ViewMode::Chart => ViewMode::Table,
        };
    }

    fn clamp_selection(&mut self) {
        if self.environments.is_empty() {
            self.table_state.select(Some(0));
        } else if self.selected() >= self.environments.len() {
            self.table_state
                .select(Some(self.environments.len() - 1));
        }
    }

    pub fn apply_sort(&mut self) {
        use crate::config::SortColumn;

        let asc = self.sort.ascending;
        self.environments.sort_by(|a, b| {
            let ord = match self.sort.column {
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Vms => a.vm_count().cmp(&b.vm_count()),
                SortColumn::Cpu => a
                    .total_cpu
                    .partial_cmp(&b.total_cpu)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Mem => a.total_mem.cmp(&b.total_mem),
                SortColumn::NetRx => a.total_net_rx.cmp(&b.total_net_rx),
                SortColumn::IoRead => a.total_blk_read.cmp(&b.total_blk_read),
                SortColumn::TimeUp => a.oldest_started_at.cmp(&b.oldest_started_at),
                SortColumn::LastChg => a.newest_started_at.cmp(&b.newest_started_at),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
    }

    fn draw(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        let last_poll = self.last_poll_secs();
        terminal.draw(|frame| {
            ui::render(
                frame,
                &self.environments,
                &mut self.table_state,
                self.view,
                &self.columns,
                &self.sort,
                self.connection,
                last_poll,
                self.show_help,
                self.hide_footer,
                &self.error_msg,
            );
        })?;
        Ok(())
    }
}
