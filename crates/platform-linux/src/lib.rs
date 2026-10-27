//! The Linux backend: reads `/proc`, `/sys` and the kernel's power-supply and sensor classes and
//! turns them into the domain types. Everything operating-system-specific sits here, so `domain`
//! stays clean and the code that displays a snapshot does not care where the numbers came from.
//!
//! The reading is deliberately boring: nothing here panics, and a machine that does not have a
//! sensor or a battery reports less, not an error.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Instant;

use domain::{
    BatteryInfo, BatteryState, CpuInfo, DiskInfo, MemoryInfo, NetworkInfo, Process, Snapshot,
    ThermalZone,
};
use sysinfo::{Components, Disks, MINIMUM_CPU_UPDATE_INTERVAL, Networks, System};

const BYTES_PER_MB: u64 = 1024 * 1024;
const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// Why a reading could not be taken at all. A missing sensor is not one of these: that is a shorter
/// snapshot, not a failure.
#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "cannot read the machine: {err}"),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
        }
    }
}

impl From<io::Error> for SnapshotError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Takes one reading of the machine.
pub fn snapshot() -> Result<Snapshot, SnapshotError> {
    let mut sys = System::new_all();
    let mut networks = Networks::new_with_refreshed_list();

    // CPU use and network speed are both the difference between two samples, so wait the shortest
    // interval `sysinfo` considers meaningful before taking the second one; without this every
    // percentage and every rate reads 0.
    let start = Instant::now();
    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();
    networks.refresh(true);
    let elapsed_seconds = start.elapsed().as_secs_f32().max(f32::EPSILON);

    Ok(Snapshot {
        cpu: cpu_info(&sys),
        memory: memory_info(&sys),
        processes: processes(&sys),
        battery: battery()?,
        thermals: thermals(),
        disks: disks(),
        networks: networks_info(&networks, elapsed_seconds),
    })
}

fn cpu_info(sys: &System) -> CpuInfo {
    CpuInfo {
        total_percent: sys.global_cpu_usage(),
        cores: sys.cpus().iter().map(|core| core.cpu_usage()).collect(),
        load_avg: load_avg(),
    }
}

/// Windows has no load average but the workspace still has to compile there, so it reads as zero.
#[cfg(unix)]
fn load_avg() -> [f32; 3] {
    let avg = System::load_average();
    [avg.one as f32, avg.five as f32, avg.fifteen as f32]
}

#[cfg(not(unix))]
fn load_avg() -> [f32; 3] {
    [0.0; 3]
}

fn memory_info(sys: &System) -> MemoryInfo {
    MemoryInfo {
        total_mb: sys.total_memory() / BYTES_PER_MB,
        used_mb: sys.used_memory() / BYTES_PER_MB,
        available_mb: sys.available_memory() / BYTES_PER_MB,
    }
}

fn processes(sys: &System) -> Vec<Process> {
    sys.processes()
        .iter()
        .map(|(pid, process)| Process {
            pid: pid.as_u32(),
            name: process.name().to_string_lossy().into_owned(),
            cpu_percent: process.cpu_usage(),
            memory_mb: process.memory() / BYTES_PER_MB,
        })
        .collect()
}

/// The first battery the kernel reports, or none at all on a machine without one (a desktop, a
/// container). Reading `/sys/class/power_supply` is the one place where a missing file means
/// "nothing to report" rather than a failure.
fn battery() -> Result<Option<BatteryInfo>, SnapshotError> {
    let entries = match fs::read_dir(POWER_SUPPLY) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    for entry in entries {
        let dir = entry?.path();
        if read_trim(&dir.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let Some(percent) = read_trim(&dir.join("capacity")).and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let state = read_trim(&dir.join("status"))
            .map_or(BatteryState::Unknown, |status| BatteryState::from_sysfs(&status));
        return Ok(Some(BatteryInfo { percent, state }));
    }

    Ok(None)
}

/// Temperature sensors, as the kernel exposes them through hwmon. Plenty of machines (and every
/// container) report none, and some need extra permissions; those simply come back empty.
fn thermals() -> Vec<ThermalZone> {
    Components::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|component| {
            let celsius = component.temperature()?;
            Some(ThermalZone {
                name: component.label().to_string(),
                celsius,
            })
        })
        .collect()
}

/// Mounted file systems with a size, biggest first. Pseudo file systems that report no size at all
/// are left out, and one device is one line: `/`, `/home` and `/var/log` are separate mounts of the
/// same btrfs file system, and repeating the same numbers four times helps nobody. The mount with
/// the shortest path (in practice `/`) is the one that stays.
fn disks() -> Vec<DiskInfo> {
    let mut disks: Vec<DiskInfo> = Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|disk| disk.total_space() > 0)
        .map(|disk| DiskInfo {
            name: disk.name().to_string_lossy().into_owned(),
            mount_point: disk.mount_point().to_string_lossy().into_owned(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            total_mb: disk.total_space() / BYTES_PER_MB,
            available_mb: disk.available_space() / BYTES_PER_MB,
        })
        .collect();

    disks.sort_by_key(|disk| disk.mount_point.len());
    disks.sort_by_key(|disk| Reverse(disk.total_mb));

    let mut seen = HashSet::new();
    disks.retain(|disk| seen.insert(disk.name.clone()));
    disks
}

/// Interfaces that moved the most bytes since boot first. The speed is what the last sampling
/// interval saw, divided by how long that interval actually was.
fn networks_info(networks: &Networks, elapsed_seconds: f32) -> Vec<NetworkInfo> {
    let mut list: Vec<NetworkInfo> = networks
        .list()
        .iter()
        .map(|(interface, network)| NetworkInfo {
            interface: interface.clone(),
            received_bytes_per_second: per_second(network.received(), elapsed_seconds),
            transmitted_bytes_per_second: per_second(network.transmitted(), elapsed_seconds),
            total_received_bytes: network.total_received(),
            total_transmitted_bytes: network.total_transmitted(),
        })
        .collect();

    list.sort_by_key(|network| Reverse(network.total_received_bytes));
    list
}

fn per_second(bytes: u64, elapsed_seconds: f32) -> u64 {
    if elapsed_seconds <= 0.0 {
        return 0;
    }
    (bytes as f32 / elapsed_seconds) as u64
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_is_the_bytes_divided_by_the_sampling_window() {
        assert_eq!(per_second(2048, 0.2), 10_240);
        assert_eq!(per_second(0, 0.2), 0);
    }

    #[test]
    fn an_impossible_window_reports_zero_instead_of_dividing_by_it() {
        assert_eq!(per_second(1024, 0.0), 0);
    }
}
