use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Status of a Vagrant environment, derived from its VMs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentStatus {
    /// All VMs running.
    Running,
    /// Some VMs running, some stopped.
    Partial,
    /// All VMs stopped / shutoff.
    Stopped,
    /// At least one VM in saved/paused state.
    Saved,
    /// At least one VM in crashed state.
    Crashed,
}

/// Per-VM snapshot of stats at a point in time.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VmSnapshot {
    pub name: String,
    pub domain_name: String,
    pub provider: String,
    pub box_name: String,
    pub state: String,
    pub running: bool,
    pub cpu_percent: f64,
    pub cpus: u32,
    pub mem_bytes: u64,
    pub mem_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub blk_read: u64,
    pub blk_write: u64,
    /// Real VM start time as Unix epoch seconds, from the libvirt PID file
    /// at /run/libvirt/qemu/<domain>.pid (birth/mtime).
    /// None if the PID file is not readable or the VM is not running.
    pub started_at: Option<u64>,
}

/// Aggregated view of a Vagrant environment (one Vagrantfile).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VagrantEnvironment {
    pub name: String,
    pub path: String,
    pub provider: String,
    pub vms: Vec<VmSnapshot>,
    pub status: EnvironmentStatus,
    // Aggregated metrics
    pub total_cpu: f64,
    pub total_mem: u64,
    pub mem_limit: u64,
    pub total_net_rx: u64,
    pub total_net_tx: u64,
    pub total_blk_read: u64,
    pub total_blk_write: u64,
    /// Oldest running VM's start time (Unix epoch seconds).
    /// Determines the TIME-UP column — how long the environment has been up.
    pub oldest_started_at: Option<u64>,
    /// Most recent state change observed (in-process Instant).
    /// Determines the LAST-CHG column — resets when vagrant-status restarts.
    pub newest_started_at: Option<Instant>,
}

impl VagrantEnvironment {
    /// Build a VagrantEnvironment from a set of VMs, computing aggregates.
    pub fn aggregate(name: String, path: String, provider: String, vms: Vec<VmSnapshot>) -> Self {
        let total_cpu: f64 = vms.iter().map(|v| v.cpu_percent).sum();
        let total_mem: u64 = vms.iter().map(|v| v.mem_bytes).sum();
        // VMs have independent allocations, just sum limits
        let mem_limit: u64 = vms.iter().map(|v| v.mem_limit).sum();
        let total_net_rx: u64 = vms.iter().map(|v| v.net_rx).sum();
        let total_net_tx: u64 = vms.iter().map(|v| v.net_tx).sum();
        let total_blk_read: u64 = vms.iter().map(|v| v.blk_read).sum();
        let total_blk_write: u64 = vms.iter().map(|v| v.blk_write).sum();

        // TIME-UP: oldest running VM's real start time (from PID file)
        let oldest_started_at = vms
            .iter()
            .filter(|v| v.running)
            .filter_map(|v| v.started_at)
            .min();

        let status = Self::derive_status(&vms);

        Self {
            name,
            path,
            provider,
            vms,
            status,
            total_cpu,
            total_mem,
            mem_limit,
            total_net_rx,
            total_net_tx,
            total_blk_read,
            total_blk_write,
            oldest_started_at,
            // LAST-CHG set by caller from the state-change tracker
            newest_started_at: None,
        }
    }

    fn derive_status(vms: &[VmSnapshot]) -> EnvironmentStatus {
        if vms.is_empty() {
            return EnvironmentStatus::Stopped;
        }

        let has_crashed = vms.iter().any(|v| v.state == "crashed");
        if has_crashed {
            return EnvironmentStatus::Crashed;
        }

        let has_saved = vms
            .iter()
            .any(|v| v.state == "saved" || v.state == "paused");
        if has_saved {
            return EnvironmentStatus::Saved;
        }

        let running_count = vms.iter().filter(|v| v.running).count();
        if running_count == vms.len() {
            EnvironmentStatus::Running
        } else if running_count > 0 {
            EnvironmentStatus::Partial
        } else {
            EnvironmentStatus::Stopped
        }
    }

    pub fn vm_count(&self) -> usize {
        self.vms.len()
    }

    pub fn mem_percent(&self) -> f64 {
        if self.mem_limit == 0 {
            0.0
        } else {
            (self.total_mem as f64 / self.mem_limit as f64) * 100.0
        }
    }
}

/// Active view mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Table,
    Chart,
}

/// Format an epoch-seconds start time as a human-readable uptime string.
/// Computes duration from `epoch_secs` to now.
pub fn uptime_str_from_epoch(epoch_secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed_secs = now.saturating_sub(epoch_secs);
    format_duration_secs(elapsed_secs)
}

/// Format an Instant-based duration as a human-readable string.
pub fn uptime_str_from_instant(instant: Instant) -> String {
    format_duration_secs(instant.elapsed().as_secs())
}

fn format_duration_secs(total_secs: u64) -> String {
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}
