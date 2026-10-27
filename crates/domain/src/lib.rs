//! The domain types: what one reading of the machine looks like. No I/O, no GUI, no operating
//! system in sight — `platform-linux` fills these in, and whatever displays them (the CLI today,
//! the Android app's screens later) only has to depend on this crate.

use serde::{Deserialize, Serialize};

/// One reading of the machine at one moment. Empty vectors and a missing battery are normal: a
/// desktop has no battery, a container has no sensors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub processes: Vec<Process>,
    pub battery: Option<BatteryInfo>,
    pub thermals: Vec<ThermalZone>,
    pub disks: Vec<DiskInfo>,
    pub networks: Vec<NetworkInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuInfo {
    /// Busy percentage of the machine as a whole.
    pub total_percent: f32,
    /// One entry per core, in the order the kernel reports them.
    pub cores: Vec<Core>,
    /// Load average over the last 1, 5 and 15 minutes.
    pub load_avg: [f32; 3],
}

/// One core: how busy it is and how fast it is running.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Core {
    pub percent: f32,
    /// Current clock in megahertz; 0 when the machine will not say.
    pub mhz: u32,
}

/// Memory and swap, in megabytes. This is the shape the helper sends on the phone and the shape
/// the desktop backend fills in, so both ends of a detailed-mode reading use the same names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryInfo {
    pub total: u64,
    pub avail: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl MemoryInfo {
    /// From the raw byte counts the kernel reports (`/proc/meminfo`).
    pub fn from_bytes(total: u64, avail: u64, cached: u64, swap_total: u64, swap_free: u64) -> Self {
        const BYTES_PER_MB: u64 = 1024 * 1024;
        Self {
            total: total / BYTES_PER_MB,
            avail: avail / BYTES_PER_MB,
            cached: cached / BYTES_PER_MB,
            swap_total: swap_total / BYTES_PER_MB,
            swap_free: swap_free / BYTES_PER_MB,
        }
    }

    pub fn used_mb(&self) -> u64 {
        self.total.saturating_sub(self.avail)
    }

    pub fn swap_used_mb(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }

    /// How much of the memory is in use, in percent. A machine that reports no memory at all
    /// (a container with nothing mounted, a kernel that will not say) counts as 0% instead of
    /// dividing by zero.
    pub fn used_percent(&self) -> f32 {
        percent(self.used_mb(), self.total)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub threads: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryInfo {
    /// Charge in percent, as the kernel (or Android) reports it.
    pub percent: f32,
    pub state: BatteryState,
    /// Battery temperature, when the machine reports one.
    pub temperature_c: Option<f32>,
}

/// What the battery is doing. `NotCharging` is a battery that is plugged in but deliberately not
/// taking a charge (Android uses it to hold a charge level); `Unknown` is a kernel that says
/// something we do not recognise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    #[default]
    Unknown,
}

impl BatteryState {
    /// Reads the state out of `/sys/class/power_supply/*/status`.
    pub fn from_sysfs(status: &str) -> Self {
        match status.trim() {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" => Self::Full,
            "Not charging" => Self::NotCharging,
            _ => Self::Unknown,
        }
    }
}

/// One temperature sensor, in degrees Celsius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalZone {
    pub name: String,
    pub celsius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_mb: u64,
    pub available_mb: u64,
}

impl DiskInfo {
    pub fn used_mb(&self) -> u64 {
        self.total_mb.saturating_sub(self.available_mb)
    }

    pub fn used_percent(&self) -> f32 {
        percent(self.used_mb(), self.total_mb)
    }
}

/// One network interface: how fast it is moving bytes right now, and how much it has moved since
/// the machine booted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub interface: String,
    pub received_bytes_per_second: u64,
    pub transmitted_bytes_per_second: u64,
    pub total_received_bytes: u64,
    pub total_transmitted_bytes: u64,
}

fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        return 0.0;
    }
    (part as f32 / whole as f32) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(total: u64, used: u64) -> MemoryInfo {
        MemoryInfo {
            total,
            avail: total - used,
            ..Default::default()
        }
    }

    fn disk(total_mb: u64, available_mb: u64) -> DiskInfo {
        DiskInfo {
            name: "/dev/sda1".to_string(),
            mount_point: "/".to_string(),
            file_system: "ext4".to_string(),
            total_mb,
            available_mb,
        }
    }

    #[test]
    fn used_percent_is_the_share_of_the_total() {
        assert_eq!(memory(8000, 2000).used_percent(), 25.0);
        assert_eq!(memory(8000, 8000).used_percent(), 100.0);
    }

    #[test]
    fn used_percent_survives_a_machine_that_reports_no_memory() {
        assert_eq!(memory(0, 0).used_percent(), 0.0);
    }

    #[test]
    fn bytes_become_megabytes_and_swap_is_counted() {
        let gib = 1024 * 1024 * 1024;
        let mem = MemoryInfo::from_bytes(8 * gib, 2 * gib, gib, 4 * gib, gib);
        assert_eq!((mem.total, mem.avail, mem.cached), (8192, 2048, 1024));
        assert_eq!(mem.used_mb(), 6144);
        assert_eq!(mem.swap_used_mb(), 3072);
    }

    #[test]
    fn a_core_carries_its_own_speed() {
        let cpu = CpuInfo {
            total_percent: 12.0,
            cores: vec![
                Core { percent: 10.0, mhz: 2016 },
                Core { percent: 14.0, mhz: 633 },
            ],
            load_avg: [0.5, 0.4, 0.3],
        };
        assert_eq!(cpu.cores[1].mhz, 633);
        assert_eq!((cpu.cores[0].percent, cpu.cores[1].percent), (10.0, 14.0));
        assert_eq!(CpuInfo::default().cores.len(), 0);
    }

    #[test]
    fn a_disk_counts_what_it_cannot_use_as_used() {
        assert_eq!(disk(1000, 250).used_mb(), 750);
        assert_eq!(disk(1000, 250).used_percent(), 75.0);
    }

    #[test]
    fn a_full_disk_is_100_percent_and_a_broken_one_is_zero() {
        assert_eq!(disk(1000, 0).used_percent(), 100.0);
        assert_eq!(disk(0, 0).used_percent(), 0.0);
    }

    #[test]
    fn a_disk_that_reports_more_free_space_than_it_has_does_not_wrap_around() {
        assert_eq!(disk(100, 200).used_mb(), 0);
        assert_eq!(disk(100, 200).used_percent(), 0.0);
    }

    #[test]
    fn battery_states_come_from_what_the_kernel_writes() {
        assert_eq!(BatteryState::from_sysfs("Charging\n"), BatteryState::Charging);
        assert_eq!(BatteryState::from_sysfs("Discharging"), BatteryState::Discharging);
        assert_eq!(BatteryState::from_sysfs("Full"), BatteryState::Full);
        assert_eq!(BatteryState::from_sysfs("Not charging"), BatteryState::NotCharging);
        assert_eq!(BatteryState::from_sysfs("something else"), BatteryState::Unknown);
    }
}
