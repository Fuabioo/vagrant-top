use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{VagrantEnvironment, VmSnapshot};

// ── Static caches ───────────────────────────────────────────────────────────

/// Cached VM info from machine-index (single-writer: only vagrant_poller calls fetch_environments).
static VM_CACHE: Mutex<Option<Vec<VmInfo>>> = Mutex::new(None);

/// Previous CPU time + wall-clock instant per domain, for delta computation.
static CPU_PREV: Mutex<Option<HashMap<String, (u64, Instant)>>> = Mutex::new(None);

/// First-observed-running time per domain (resets on restart).
static UPTIME_TRACKER: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Last state per domain, for detecting state transitions.
static LAST_STATE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Last state change time per domain.
static LAST_CHANGE: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

// ── Machine index types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MachineIndex {
    #[allow(dead_code)]
    version: u32,
    machines: HashMap<String, MachineEntry>,
}

#[derive(Debug, Deserialize)]
struct MachineEntry {
    #[allow(dead_code)]
    local_data_path: Option<String>,
    name: String,
    provider: String,
    state: Option<String>,
    vagrantfile_path: Option<String>,
    extra_data: Option<ExtraData>,
    #[allow(dead_code)]
    vagrantfile_name: Option<String>,
    #[allow(dead_code)]
    updated_at: Option<String>,
    #[allow(dead_code)]
    architecture: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtraData {
    #[serde(rename = "box")]
    box_info: Option<BoxInfo>,
}

#[derive(Debug, Deserialize)]
struct BoxInfo {
    name: Option<String>,
    #[allow(dead_code)]
    provider: Option<String>,
    #[allow(dead_code)]
    architecture: Option<String>,
    #[allow(dead_code)]
    version: Option<String>,
}

// ── Internal VM info ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct VmInfo {
    name: String,
    domain_name: String,
    provider: String,
    state: String,
    environment: String,
    vagrantfile_path: String,
    box_name: String,
}

// ── DomStats ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct DomStats {
    state: u32,
    cpu_time: u64,
    vcpu_current: u32,
    balloon_rss: Option<u64>,
    balloon_maximum: u64,
    balloon_current: u64,
    net_rx_bytes: u64,
    net_tx_bytes: u64,
    blk_rd_bytes: u64,
    blk_wr_bytes: u64,
}

// ── Provider support ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProviderSupport {
    pub has_virsh: bool,
}

// ── Mutex helpers ───────────────────────────────────────────────────────────

fn lock_vm_cache() -> std::sync::MutexGuard<'static, Option<Vec<VmInfo>>> {
    VM_CACHE.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_cpu_prev() -> std::sync::MutexGuard<'static, Option<HashMap<String, (u64, Instant)>>> {
    CPU_PREV.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_uptime() -> std::sync::MutexGuard<'static, Option<HashMap<String, Instant>>> {
    UPTIME_TRACKER.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_last_state() -> std::sync::MutexGuard<'static, Option<HashMap<String, String>>> {
    LAST_STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_last_change() -> std::sync::MutexGuard<'static, Option<HashMap<String, Instant>>> {
    LAST_CHANGE.lock().unwrap_or_else(|p| p.into_inner())
}

// ── Layer 1: Machine index ──────────────────────────────────────────────────

/// Find the machine-index file. Checks $VAGRANT_HOME first, then ~/.vagrant.d.
pub fn find_machine_index() -> Option<PathBuf> {
    let vagrant_home = if let Ok(home) = std::env::var("VAGRANT_HOME") {
        PathBuf::from(home)
    } else {
        dirs::home_dir()?.join(".vagrant.d")
    };

    let path = vagrant_home.join("data/machine-index/index");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Read and parse the machine-index file.
fn read_machine_index(path: &Path) -> Result<Vec<VmInfo>> {
    let content = std::fs::read_to_string(path).context("Failed to read machine-index")?;
    let index: MachineIndex =
        serde_json::from_str(&content).context("Failed to parse machine-index JSON")?;

    if index.version != 1 {
        tracing::warn!(
            "Machine index version {} (expected 1), attempting parse anyway",
            index.version
        );
    }

    let mut vms = Vec::new();

    for (_id, entry) in &index.machines {
        let vagrantfile_path = entry.vagrantfile_path.clone().unwrap_or_default();
        let environment = Path::new(&vagrantfile_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| vagrantfile_path.clone());

        let domain_name = derive_domain_name(&vagrantfile_path, &entry.name);

        let box_name = entry
            .extra_data
            .as_ref()
            .and_then(|ed| ed.box_info.as_ref())
            .and_then(|b| b.name.clone())
            .unwrap_or_default();

        let state = entry.state.clone().unwrap_or_else(|| "unknown".to_string());

        vms.push(VmInfo {
            name: entry.name.clone(),
            domain_name,
            provider: entry.provider.clone(),
            state,
            environment,
            vagrantfile_path,
            box_name,
        });
    }

    Ok(vms)
}

/// Derive libvirt domain name: <dir_name>_<vm_name>
fn derive_domain_name(vagrantfile_path: &str, vm_name: &str) -> String {
    let dir = Path::new(vagrantfile_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    format!("{}_{}", dir, vm_name)
}

/// Fallback: resolve domain name from UUID file via `virsh domname`.
#[allow(dead_code)]
async fn resolve_domain_via_uuid(local_data_path: &str, vm_name: &str, provider: &str) -> Option<String> {
    let id_path = PathBuf::from(local_data_path)
        .join("machines")
        .join(vm_name)
        .join(provider)
        .join("id");

    let uuid = tokio::fs::read_to_string(&id_path).await.ok()?;
    let uuid = uuid.trim();
    if uuid.is_empty() {
        return None;
    }

    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("virsh")
            .args(["domname", uuid])
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    None
}

// ── Layer 2: Provider queries ───────────────────────────────────────────────

/// Detect available providers by running `virsh version`.
pub async fn detect_providers() -> ProviderSupport {
    let has_virsh = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("virsh")
            .arg("version")
            .output(),
    )
    .await
    .map(|r| r.map(|o| o.status.success()).unwrap_or(false))
    .unwrap_or(false);

    ProviderSupport { has_virsh }
}

/// Fetch domstats for all running domains in a single virsh call.
async fn fetch_all_domstats() -> Result<HashMap<String, DomStats>> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("virsh")
            .arg("domstats")
            .output(),
    )
    .await
    .context("virsh domstats timed out")?
    .context("Failed to run virsh domstats")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("virsh domstats failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_domstats_output(&stdout)
}

/// Parse multi-domain domstats output.
fn parse_domstats_output(output: &str) -> Result<HashMap<String, DomStats>> {
    let mut result: HashMap<String, DomStats> = HashMap::new();
    let mut current_domain: Option<String> = None;
    let mut current_stats = DomStats::default();

    // Track counts for indexed fields
    let mut net_count: u64 = 0;
    let mut block_count: u64 = 0;

    for line in output.lines() {
        if let Some(name) = line.strip_prefix("Domain: '").and_then(|s| s.strip_suffix('\'')) {
            // Save previous domain if any
            if let Some(domain) = current_domain.take() {
                result.insert(domain, current_stats);
            }
            current_domain = Some(name.to_string());
            current_stats = DomStats::default();
            net_count = 0;
            block_count = 0;
            continue;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, val_str)) = line.split_once('=') else {
            continue;
        };

        // Parse value as u64, skip non-numeric values
        let Ok(val) = val_str.parse::<u64>() else {
            continue;
        };

        match key {
            "state.state" => current_stats.state = val as u32,
            "cpu.time" => current_stats.cpu_time = val,
            "vcpu.current" => current_stats.vcpu_current = val as u32,
            "balloon.rss" => current_stats.balloon_rss = Some(val),
            "balloon.maximum" => current_stats.balloon_maximum = val,
            "balloon.current" => current_stats.balloon_current = val,
            "net.count" => net_count = val,
            "block.count" => block_count = val,
            _ => {
                // Handle indexed network fields: net.N.rx.bytes, net.N.tx.bytes
                for n in 0..net_count {
                    if key == format!("net.{}.rx.bytes", n) {
                        current_stats.net_rx_bytes += val;
                    } else if key == format!("net.{}.tx.bytes", n) {
                        current_stats.net_tx_bytes += val;
                    }
                }
                // Handle indexed block fields: block.N.rd.bytes, block.N.wr.bytes
                for n in 0..block_count {
                    if key == format!("block.{}.rd.bytes", n) {
                        current_stats.blk_rd_bytes += val;
                    } else if key == format!("block.{}.wr.bytes", n) {
                        current_stats.blk_wr_bytes += val;
                    }
                }
            }
        }
    }

    // Save last domain
    if let Some(domain) = current_domain {
        result.insert(domain, current_stats);
    }

    Ok(result)
}

// ── Layer 3: Aggregation ────────────────────────────────────────────────────

/// Fetch Vagrant environments. If `relist` is true, re-reads the machine index.
pub async fn fetch_environments(
    index_path: &Path,
    provider_support: &ProviderSupport,
    relist: bool,
) -> Result<Vec<VagrantEnvironment>> {
    // Re-read machine index on relist
    if relist || lock_vm_cache().is_none() {
        let vms = read_machine_index(index_path)?;

        // Prune CPU_PREV and UPTIME_TRACKER for disappeared domains
        let current_domains: std::collections::HashSet<String> =
            vms.iter().map(|v| v.domain_name.clone()).collect();

        {
            let mut cpu_guard = lock_cpu_prev();
            if let Some(map) = cpu_guard.as_mut() {
                map.retain(|k, _| current_domains.contains(k));
            }
        }
        {
            let mut uptime_guard = lock_uptime();
            if let Some(map) = uptime_guard.as_mut() {
                map.retain(|k, _| current_domains.contains(k));
            }
        }
        {
            let mut state_guard = lock_last_state();
            if let Some(map) = state_guard.as_mut() {
                map.retain(|k, _| current_domains.contains(k));
            }
        }
        {
            let mut change_guard = lock_last_change();
            if let Some(map) = change_guard.as_mut() {
                map.retain(|k, _| current_domains.contains(k));
            }
        }

        *lock_vm_cache() = Some(vms);
    }

    let vms = lock_vm_cache().clone().unwrap_or_default();
    if vms.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch domstats (single call for all domains)
    let domstats = if provider_support.has_virsh {
        match fetch_all_domstats().await {
            Ok(stats) => stats,
            Err(e) => {
                tracing::warn!("virsh domstats failed: {}", e);
                HashMap::new()
            }
        }
    } else {
        HashMap::new()
    };

    let now = Instant::now();

    // Build snapshots
    let mut snapshots: Vec<(String, VmSnapshot)> = Vec::new();

    for vm in &vms {
        let stats = domstats.get(&vm.domain_name);

        // Determine real-time state from domstats if available
        let effective_state = if let Some(s) = stats {
            match s.state {
                1 => "running".to_string(),
                3 => "paused".to_string(),
                5 => "shutoff".to_string(),
                6 => "crashed".to_string(),
                _ => vm.state.clone(),
            }
        } else {
            vm.state.clone()
        };

        let running = effective_state == "running";

        // Track uptime (first-observed running)
        {
            let mut uptime_guard = lock_uptime();
            let map = uptime_guard.get_or_insert_with(HashMap::new);
            if running {
                map.entry(vm.domain_name.clone()).or_insert(now);
            } else {
                map.remove(&vm.domain_name);
            }
        }

        // Track state changes
        {
            let mut state_guard = lock_last_state();
            let state_map = state_guard.get_or_insert_with(HashMap::new);
            let prev_state = state_map.get(&vm.domain_name);
            if prev_state.map(|s| s != &effective_state).unwrap_or(true) {
                // State changed
                let mut change_guard = lock_last_change();
                let change_map = change_guard.get_or_insert_with(HashMap::new);
                change_map.insert(vm.domain_name.clone(), now);
            }
            state_map.insert(vm.domain_name.clone(), effective_state.clone());
        }

        let started_at = lock_uptime()
            .as_ref()
            .and_then(|m| m.get(&vm.domain_name).copied());

        let (cpu_percent, cpus, mem_bytes, mem_limit, net_rx, net_tx, blk_read, blk_write) =
            if let Some(s) = stats {
                // CPU% delta computation
                let cpu_percent = {
                    let mut cpu_guard = lock_cpu_prev();
                    let map = cpu_guard.get_or_insert_with(HashMap::new);
                    let percent = if let Some(&(prev_time, prev_instant)) =
                        map.get(&vm.domain_name)
                    {
                        let cpu_delta_ns = s.cpu_time.saturating_sub(prev_time);
                        let wall_delta_ns = now
                            .duration_since(prev_instant)
                            .as_nanos() as u64;
                        if wall_delta_ns > 0 {
                            (cpu_delta_ns as f64 / wall_delta_ns as f64) * 100.0
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    map.insert(vm.domain_name.clone(), (s.cpu_time, now));
                    percent
                };

                // Memory: prefer balloon.rss, fallback to balloon.current
                let mem_bytes = s
                    .balloon_rss
                    .unwrap_or(s.balloon_current)
                    * 1024; // KiB -> bytes
                let mem_limit = s.balloon_maximum * 1024;

                (
                    cpu_percent,
                    s.vcpu_current,
                    mem_bytes,
                    mem_limit,
                    s.net_rx_bytes,
                    s.net_tx_bytes,
                    s.blk_rd_bytes,
                    s.blk_wr_bytes,
                )
            } else {
                // No domstats: zeroed
                (0.0, 0, 0, 0, 0, 0, 0, 0)
            };

        let snap = VmSnapshot {
            name: vm.name.clone(),
            domain_name: vm.domain_name.clone(),
            provider: vm.provider.clone(),
            box_name: vm.box_name.clone(),
            state: effective_state,
            running,
            cpu_percent,
            cpus,
            mem_bytes,
            mem_limit,
            net_rx,
            net_tx,
            blk_read,
            blk_write,
            started_at,
        };

        snapshots.push((vm.environment.clone(), snap));
    }

    // Group by environment
    let mut groups: HashMap<String, (String, String, Vec<VmSnapshot>)> = HashMap::new();
    for (env_name, snap) in snapshots {
        let vm = vms
            .iter()
            .find(|v| v.domain_name == snap.domain_name)
            .unwrap();
        groups
            .entry(env_name)
            .or_insert_with(|| (vm.vagrantfile_path.clone(), vm.provider.clone(), Vec::new()))
            .2
            .push(snap);
    }

    // Aggregate into environments
    let environments: Vec<VagrantEnvironment> = groups
        .into_iter()
        .map(|(name, (path, provider, vms))| {
            let mut env = VagrantEnvironment::aggregate(name, path, provider, vms);
            // Set newest_started_at from LAST_CHANGE tracker
            let change_guard = lock_last_change();
            if let Some(change_map) = change_guard.as_ref() {
                let latest_change = env
                    .vms
                    .iter()
                    .filter_map(|v| change_map.get(&v.domain_name).copied())
                    .max();
                if latest_change.is_some() {
                    env.newest_started_at = latest_change;
                }
            }
            env
        })
        .collect();

    Ok(environments)
}

// ── Connection state ────────────────────────────────────────────────────────

/// Three-state connection model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Index readable + virsh works.
    Connected,
    /// Index readable + virsh fails.
    IndexOnly,
    /// Index not found / unreadable.
    Disconnected,
}
