mod app;
mod config;
mod event;
mod model;
mod ui;
mod vagrant;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self as ct_event, Event};
use tokio::sync::mpsc;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter};

use app::App;
use event::AppEvent;

#[tokio::main]
async fn main() -> Result<()> {
    // File-based logging (TUI owns stdout)
    let file_appender = rolling::never(".", "vagrant-status.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    tracing::info!("vagrant-status starting");

    // Restore terminal on panic so it doesn't get stuck in raw mode
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        default_hook(info);
    }));

    let (tx, rx) = mpsc::channel::<AppEvent>(256);
    let running = Arc::new(AtomicBool::new(true));

    // Spawn input reader on a blocking thread
    let tx_input = tx.clone();
    let running_input = running.clone();
    tokio::task::spawn_blocking(move || {
        input_reader(tx_input, running_input);
    });

    // Spawn render ticker
    let tx_tick = tx.clone();
    tokio::spawn(async move {
        tick_producer(tx_tick).await;
    });

    // Spawn vagrant poller
    let tx_vagrant = tx.clone();
    tokio::spawn(async move {
        vagrant_poller(tx_vagrant).await;
    });

    // Drop the original sender so only tasks hold senders
    drop(tx);

    // Init terminal
    let mut terminal = ratatui::init();

    // Parse CLI flags
    let hide_help = std::env::args().any(|a| a == "--hide-help");

    // Run app
    let mut app = App::new(hide_help);
    let result = app.run(&mut terminal, rx).await;

    // Signal the blocking input reader to stop
    running.store(false, Ordering::Relaxed);

    // Restore terminal
    ratatui::restore();

    result
}

/// Reads crossterm events on a blocking thread, forwards to channel.
fn input_reader(tx: mpsc::Sender<AppEvent>, running: Arc<AtomicBool>) {
    while running.load(Ordering::Relaxed) {
        match ct_event::poll(Duration::from_millis(100)) {
            Ok(true) => match ct_event::read() {
                Ok(Event::Key(key)) => {
                    if tx.blocking_send(AppEvent::Key(key)).is_err() {
                        return;
                    }
                }
                Ok(Event::Resize(w, h)) => {
                    if tx.blocking_send(AppEvent::Resize(w, h)).is_err() {
                        return;
                    }
                }
                _ => {}
            },
            Ok(false) => {}
            Err(_) => return,
        }
    }
}

/// Sends Tick events at ~30fps.
async fn tick_producer(tx: mpsc::Sender<AppEvent>) {
    let mut interval = tokio::time::interval(Duration::from_millis(33));
    loop {
        interval.tick().await;
        if tx.send(AppEvent::Tick).await.is_err() {
            return;
        }
    }
}

/// Polls Vagrant for VM data. Reconnects on failure.
async fn vagrant_poller(tx: mpsc::Sender<AppEvent>) {
    let stats_interval = Duration::from_secs(2);
    let list_interval = Duration::from_secs(5);
    let reconnect_delay = Duration::from_secs(3);

    loop {
        // Step 1: Locate machine index
        let index_path = match vagrant::find_machine_index() {
            Some(p) => {
                tracing::info!("Found machine index at {:?}", p);
                p
            }
            None => {
                let _ = tx
                    .send(AppEvent::VagrantError {
                        message: "Vagrant machine index not found. Is Vagrant installed?"
                            .to_string(),
                        index_available: false,
                    })
                    .await;
                tokio::time::sleep(reconnect_delay).await;
                continue;
            }
        };

        // Step 2: Detect virsh availability
        let provider_support = vagrant::detect_providers().await;
        if !provider_support.has_virsh {
            tracing::warn!("virsh not available, running in Index Only mode");
        }

        // Step 3: Poll loop
        let mut last_list = Instant::now() - list_interval;
        let mut interval = tokio::time::interval(stats_interval);

        loop {
            interval.tick().await;

            let should_relist = last_list.elapsed() >= list_interval;
            if should_relist {
                last_list = Instant::now();
            }

            match vagrant::fetch_environments(&index_path, &provider_support, should_relist).await {
                Ok(environments) => {
                    if tx
                        .send(AppEvent::VagrantUpdate(environments))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!("Vagrant poll error: {}", e);
                    // Check if index is still available
                    let index_available = index_path.exists();
                    let _ = tx
                        .send(AppEvent::VagrantError {
                            message: format!("{}", e),
                            index_available,
                        })
                        .await;
                    break;
                }
            }
        }

        tokio::time::sleep(reconnect_delay).await;
    }
}
