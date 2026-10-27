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
    BatteryHealth, BatteryInfo, BatteryState, Core, CpuInfo, DiskInfo, MemoryInfo, NetworkInfo,
    Process, Snapshot, ThermalZone,
};
use linux_proc::{CpuTimes, cpu_percent, cpu_times, load_average, meminfo};
use sysinfo::{Components, Disks, MINIMUM_CPU_UPDATE_INTERVAL, Networks, System};

const BYTES_PER_MB: u64 = 1024 * 1024;
const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// Why a reading could not be taken at all. A missing sensor or battery is not one of these: that is
/// a shorter snapshot, not a failure.
#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    /// The kernel would not say how much memory the machine has.
    NoMemory,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "cannot read the machine: {err}"),
            Self::NoMemory => write!(formatter, "{} is not readable", linux_proc::MEMINFO),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::NoMemory => None,
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
    // CPU use is the difference between two readings of /proc/stat, so the first one is taken
    // before the wait below.
    let cpu_before = cpu_times();

    let mut sys = System::new_all();
    let mut networks = Networks::new_with_refreshed_list();

    // CPU use and network speed are both the difference between two samples, so wait the shortest
    // interval `sysinfo` considers meaningful before taking the second one; without this every
    // percentage and every rate reads 0.
    let start = Instant::now();
    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    let cpu_after = cpu_times();
    sys.refresh_all();
    networks.refresh(true);
    let elapsed_seconds = start.elapsed().as_secs_f32().max(f32::EPSILON);

    Ok(Snapshot {
        cpu: cpu_info(&cpu_before, &cpu_after, &sys),
        memory: memory_info()?,
        processes: processes(&sys),
        battery: battery()?,
        thermals: thermals(),
        disks: disks(),
        networks: networks_info(&networks, elapsed_seconds),
    })
}

/// `/proc/stat` writes the machine as a whole first and then one line per core, and `/proc/loadavg`
/// the load average. Both are read by `linux-proc`, the same way the Android app reads them.
fn cpu_info(before: &[CpuTimes], after: &[CpuTimes], sys: &System) -> CpuInfo {
    let percent = |index: usize| match (before.get(index), after.get(index)) {
        (Some(previous), Some(current)) => cpu_percent(*previous, *current).unwrap_or(0.0),
        _ => 0.0,
    };

    let cores = (1..after.len())
        .map(|index| Core {
            percent: Some(percent(index)),
            // `sysinfo` reads the clock from /sys/devices/system/cpu/*/cpufreq; a kernel that keeps
            // the file to itself reports 0, which is a clock we do not know rather than a stopped core.
            mhz: sys
                .cpus()
                .get(index - 1)
                .map(|core| core.frequency())
                .filter(|khz| *khz > 0)
                .and_then(|khz| u32::try_from(khz).ok()),
            // `sysinfo` knows the clock but not the range it came from.
            max_mhz: None,
        })
        .collect();

    CpuInfo {
        total_percent: Some(percent(0)),
        cores,
        load_avg: load_average().unwrap_or([0.0; 3]),
    }
}

/// `/proc/meminfo`: the same file, read and parsed the same way as the app's basic mode.
fn memory_info() -> Result<MemoryInfo, SnapshotError> {
    let mem = meminfo().ok_or(SnapshotError::NoMemory)?;
    Ok(MemoryInfo::from_bytes(
        mem.total,
        mem.avail,
        mem.cached,
        mem.swap_total,
        mem.swap_free,
    ))
}

fn processes(sys: &System) -> Vec<Process> {
    sys.processes()
        .iter()
        .map(|(pid, process)| Process {
            pid: pid.as_u32(),
            name: process.name().to_string_lossy().into_owned(),
            cpu_percent: process.cpu_usage(),
            memory_mb: process.memory() / BYTES_PER_MB,
            threads: process.tasks().map_or(1, |tasks| tasks.len() as u32),
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
        // The kernel reports the battery temperature in tenths of a degree.
        let temperature_c = read_trim(&dir.join("temp"))
            .and_then(|value| value.parse::<f32>().ok())
            .map(|tenths| tenths / 10.0);
        let number = |name: &str| read_trim(&dir.join(name)).and_then(|value| value.parse::<i64>().ok());
        return Ok(Some(BatteryInfo {
            percent,
            state,
            temperature_c,
            // Voltage and current are microvolts and microamps here; the direction of the current
            // is what `state` says, so only its size is kept. Not every battery reports either.
            voltage_mv: number("voltage_now").filter(|micro| *micro > 0).map(|micro| (micro / 1000) as u32),
            current_ma: number("current_now").map(|micro| (micro.unsigned_abs() / 1000) as u32),
            cycles: number("cycle_count").filter(|count| *count > 0).map(|count| count as u32),
            health: read_trim(&dir.join("health"))
                .map_or(BatteryHealth::Unknown, |health| BatteryHealth::from_sysfs(&health)),
            // What is left is Android's: `/sys` says whether a supply is online, not whether it is a
            // wall plug or a laptop, and only Android estimates how long a charge still needs.
            ..Default::default()
        }));
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
