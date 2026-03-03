use crossterm::event::KeyEvent;

use crate::model::VagrantEnvironment;

/// Events flowing through the application's main channel.
#[derive(Debug)]
#[allow(dead_code)]
pub enum AppEvent {
    /// A keyboard event from the terminal.
    Key(KeyEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Render tick (~30fps).
    Tick,
    /// Fresh Vagrant data arrived.
    VagrantUpdate(Vec<VagrantEnvironment>),
    /// Vagrant data source error.
    /// `index_available` distinguishes IndexOnly (virsh failed but index readable)
    /// from Disconnected (index not found).
    VagrantError {
        message: String,
        index_available: bool,
    },
}
