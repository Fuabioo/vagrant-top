use std::time::{Duration, Instant};

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
    pub started_at: Option<Instant>,
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
    /// Oldest running VM's first-observed time.
    pub oldest_started_at: Option<Instant>,
    /// Most recent state change observed.
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

        let running_starts: Vec<Instant> = vms
            .iter()
            .filter(|v| v.running)
            .filter_map(|v| v.started_at)
            .collect();
        let oldest_started_at = running_starts.iter().min().copied();

        let all_starts: Vec<Instant> = vms.iter().filter_map(|v| v.started_at).collect();
        let newest_started_at = all_starts.iter().max().copied();

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
            newest_started_at,
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

/// Format a duration as a human-readable string.
pub fn uptime_str(elapsed: Duration) -> String {
    let total_secs = elapsed.as_secs();

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
