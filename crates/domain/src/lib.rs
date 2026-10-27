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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuInfo {
    /// Busy percentage of the machine as a whole.
    pub total_percent: f32,
    /// Busy percentage of each core, in the order the kernel reports them.
    pub cores: Vec<f32>,
    /// Load average over the last 1, 5 and 15 minutes.
    pub load_avg: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_mb: u64,
    pub used_mb: u64,
    pub available_mb: u64,
}

impl MemoryInfo {
    /// How much of the memory is in use, in percent. A machine that reports no memory at all
    /// (a container with nothing mounted, a kernel that will not say) counts as 0% instead of
    /// dividing by zero.
    pub fn used_percent(&self) -> f32 {
        percent(self.used_mb, self.total_mb)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryInfo {
    /// Charge in percent, as the kernel reports it.
    pub percent: f32,
    pub state: BatteryState,
}

/// What the battery is doing. `NotCharging` is a battery that is plugged in but deliberately not
/// taking a charge (Android uses it to hold a charge level); `Unknown` is a kernel that says
/// something we do not recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    Charging,
    Discharging,
    Full,
    NotCharging,
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

    fn memory(total_mb: u64, used_mb: u64) -> MemoryInfo {
        MemoryInfo {
            total_mb,
            used_mb,
            available_mb: total_mb - used_mb,
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
