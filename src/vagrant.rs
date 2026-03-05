use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{VagrantEnvironment, VmSnapshot};

// ── Static caches ───────────────────────────────────────────────────────────

/// Cached VM info from machine-index (single-writer: only vagrant_poller calls fetch_environments).
static VM_CACHE: Mutex<Option<Vec<VmInfo>>> = Mutex::new(None);

/// Previous CPU time + wall-clock instant per domain, for delta computation.
static CPU_PREV: Mutex<Option<HashMap<String, (u64, Instant)>>> = Mutex::new(None);

/// Last state per domain, for detecting state transitions (LAST-CHG column).
/// Keyed by domain name. On the first observation, the state is recorded but
/// no "change" is emitted — LAST-CHG only fires on actual transitions.
static LAST_STATE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Last state change time per domain (in-process Instant).
/// Resets when vagrant-status restarts. Only updated on actual state transitions,
/// NOT on first observation (avoids showing "0m" for every VM at startup).
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
    /// Resolved libvirt connection URI (e.g. "qemu:///system").
    /// Passed as `--connect <uri>` to every virsh call so we don't depend
    /// on LIBVIRT_DEFAULT_URI being in the environment (which may be absent
    /// when launched from HUDs, systemd units, or other minimal contexts).
    pub virsh_uri: Option<String>,
}

// ── Mutex helpers ───────────────────────────────────────────────────────────

fn lock_vm_cache() -> std::sync::MutexGuard<'static, Option<Vec<VmInfo>>> {
    VM_CACHE.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_cpu_prev() -> std::sync::MutexGuard<'static, Option<HashMap<String, (u64, Instant)>>> {
    CPU_PREV.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_last_state() -> std::sync::MutexGuard<'static, Option<HashMap<String, String>>> {
    LAST_STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_last_change() -> std::sync::MutexGuard<'static, Option<HashMap<String, Instant>>> {
    LAST_CHANGE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Build a virsh Command with the resolved connection URI.
///
/// Always passes `--connect <uri>` explicitly rather than relying on the
/// `LIBVIRT_DEFAULT_URI` env var, because the parent process (e.g. dev-hud,
/// systemd, or a Wayland compositor) may not have that variable set.
/// Without it, virsh defaults to `qemu:///session` which is the wrong daemon.
fn virsh_command(provider: &ProviderSupport) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("virsh");
    if let Some(uri) = &provider.virsh_uri {
        cmd.args(["--connect", uri]);
    }
    cmd
}

// ── VM start time from PID file ─────────────────────────────────────────────

/// Get the real VM start time by reading the libvirt PID file's modification time.
///
/// Libvirt creates `/run/libvirt/qemu/<domain>.pid` when a domain starts.
/// The file's mtime (and birth time) correspond to the VM boot moment.
/// This file is world-readable (0644), so no root access needed.
///
/// Returns Unix epoch seconds, or None if the file doesn't exist / can't be read.
fn get_domain_start_time(domain_name: &str) -> Option<u64> {
    let pid_path = PathBuf::from("/run/libvirt/qemu")
        .join(format!("{}.pid", domain_name));

    let metadata = std::fs::metadata(&pid_path).ok()?;

    // Prefer mtime (always available on Linux); it equals the birth time for
    // PID files since libvirt writes them once at domain startup and never modifies them.
    let mtime = metadata.modified().ok()?;
    let epoch_secs = mtime.duration_since(UNIX_EPOCH).ok()?.as_secs();

    Some(epoch_secs)
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
async fn resolve_domain_via_uuid(
    provider: &ProviderSupport,
    local_data_path: &str,
    vm_name: &str,
    vm_provider: &str,
) -> Option<String> {
    let id_path = PathBuf::from(local_data_path)
        .join("machines")
        .join(vm_name)
        .join(vm_provider)
        .join("id");

    let uuid = tokio::fs::read_to_string(&id_path).await.ok()?;
    let uuid = uuid.trim();
    if uuid.is_empty() {
        return None;
    }

    let mut cmd = virsh_command(provider);
    cmd.args(["domname", uuid]);

    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
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

/// Detect available providers and resolve the libvirt connection URI.
///
/// Resolution order for URI:
/// 1. `$LIBVIRT_DEFAULT_URI` environment variable (trusted, used as-is)
/// 2. Probe: try `qemu:///system` first (where vagrant-libvirt VMs live),
///    then fall back to `virsh uri` default.
///
/// The probe is necessary because without LIBVIRT_DEFAULT_URI, `virsh uri`
/// returns `qemu:///session` (user daemon), which has no vagrant VMs.
/// Vagrant-libvirt always uses the system daemon.
pub async fn detect_providers() -> ProviderSupport {
    let virsh_uri = resolve_virsh_uri().await;

    let has_virsh = if let Some(uri) = &virsh_uri {
        virsh_version_ok(Some(uri)).await
    } else {
        virsh_version_ok(None).await
    };

    if has_virsh {
        tracing::info!("virsh available, URI: {:?}", virsh_uri);
    } else {
        tracing::warn!("virsh not available or failed with URI: {:?}", virsh_uri);
    }

    ProviderSupport {
        has_virsh,
        virsh_uri,
    }
}

/// Run `virsh version` with an optional --connect URI. Returns true if it succeeds.
async fn virsh_version_ok(uri: Option<&str>) -> bool {
    let mut cmd = tokio::process::Command::new("virsh");
    if let Some(u) = uri {
        cmd.args(["--connect", u]);
    }
    cmd.arg("version");

    tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .map(|r| r.map(|o| o.status.success()).unwrap_or(false))
        .unwrap_or(false)
}

/// Check if `virsh domstats` returns any domains with the given URI.
async fn virsh_has_domains(uri: &str) -> bool {
    let mut cmd = tokio::process::Command::new("virsh");
    cmd.args(["--connect", uri, "domstats"]);

    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await else {
        return false;
    };

    output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains("Domain: '")
}

/// Resolve the libvirt connection URI.
async fn resolve_virsh_uri() -> Option<String> {
    // 1. Explicit env var — trust it unconditionally
    if let Ok(uri) = std::env::var("LIBVIRT_DEFAULT_URI") {
        if !uri.is_empty() {
            tracing::info!("Using LIBVIRT_DEFAULT_URI from environment: {}", uri);
            return Some(uri);
        }
    }

    // 2. Probe qemu:///system (where vagrant-libvirt VMs always live).
    //    This is the common case when the env var is missing (dev-hud, systemd, etc.)
    let system_uri = "qemu:///system";
    if virsh_has_domains(system_uri).await {
        tracing::info!("Probed {} — found domains, using it", system_uri);
        return Some(system_uri.to_string());
    }

    // 3. Even if no domains are running right now on system, prefer it if it connects
    if virsh_version_ok(Some(system_uri)).await {
        tracing::info!("Probed {} — connectable (no domains yet), using it", system_uri);
        return Some(system_uri.to_string());
    }

    // 4. Last resort: ask virsh for its default (likely qemu:///session)
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("virsh")
            .arg("uri")
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let uri = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !uri.is_empty() {
            tracing::info!("Falling back to `virsh uri` default: {}", uri);
            return Some(uri);
        }
    }

    None
}

/// Fetch domstats for all running domains in a single virsh call.
async fn fetch_all_domstats(provider: &ProviderSupport) -> Result<HashMap<String, DomStats>> {
    let mut cmd = virsh_command(provider);
    cmd.arg("domstats");

    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
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

        // Prune caches for domains that disappeared from the machine index
        let current_domains: std::collections::HashSet<String> =
            vms.iter().map(|v| v.domain_name.clone()).collect();

        {
            let mut cpu_guard = lock_cpu_prev();
            if let Some(map) = cpu_guard.as_mut() {
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
        match fetch_all_domstats(provider_support).await {
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

        // Track state changes for LAST-CHG column.
        // On first observation we record the state but do NOT emit a change event,
        // so LAST-CHG starts as "-" rather than "0m" for pre-existing VMs.
        {
            let mut state_guard = lock_last_state();
            let state_map = state_guard.get_or_insert_with(HashMap::new);
            let prev_state = state_map.get(&vm.domain_name);
            match prev_state {
                Some(prev) if prev != &effective_state => {
                    // Actual state transition — record change time
                    let mut change_guard = lock_last_change();
                    let change_map = change_guard.get_or_insert_with(HashMap::new);
                    change_map.insert(vm.domain_name.clone(), now);
                }
                None => {
                    // First observation — just record state, no change event
                }
                _ => {
                    // Same state — no change
                }
            }
            state_map.insert(vm.domain_name.clone(), effective_state.clone());
        }

        // TIME-UP: read real VM start time from the libvirt PID file.
        // This gives the actual boot time (Unix epoch seconds), surviving
        // vagrant-status restarts. Falls back to None if the PID file
        // is missing (VM not running or no libvirt access).
        let started_at = if running {
            get_domain_start_time(&vm.domain_name)
        } else {
            None
        };

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
            // Set newest_started_at from LAST_CHANGE tracker (in-process Instants)
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
