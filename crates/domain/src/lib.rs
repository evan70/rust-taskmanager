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
    /// Terminal voltage in millivolts.
    pub voltage_mv: Option<u32>,
    /// Current in milliamps, always a magnitude: which way it flows is what `state` says.
    pub current_ma: Option<u32>,
    /// How many times the battery has been charged, on phones that keep the count.
    pub cycles: Option<u32>,
    pub health: BatteryHealth,
    /// What the phone is plugged into, when it is plugged in at all.
    pub plugged: Plugged,
    /// Android's estimate of how long the charge still needs.
    pub full_in_ms: Option<u64>,
}

impl BatteryInfo {
    pub fn is_charging(&self) -> bool {
        self.state == BatteryState::Charging
    }

    pub fn is_full(&self) -> bool {
        self.state == BatteryState::Full
    }

    /// Power in watts, when both voltage and current are known.
    pub fn watts(&self) -> Option<f32> {
        Some(self.voltage_mv? as f32 * self.current_ma? as f32 / 1_000_000.0)
    }
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

    /// Reads the state out of Android's `BatteryManager.EXTRA_STATUS`, which counts charges and
    /// discharges rather than naming them.
    pub fn from_android(status: i64) -> Self {
        match status {
            2 => Self::Charging,
            3 => Self::Discharging,
            4 => Self::NotCharging,
            5 => Self::Full,
            _ => Self::Unknown,
        }
    }

    /// What the state is called on screen. `NotCharging` is a phone that is plugged in and holding
    /// its level, which the app has always shown as being on battery.
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Charging => "Charging",
            _ => "On battery",
        }
    }
}

/// What the battery says about itself. `Overheat` and `Cold` are the battery's own temperature
/// limits, which have nothing to do with the phone's thermal status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryHealth {
    Good,
    Overheat,
    Dead,
    OverVoltage,
    Failure,
    Cold,
    #[default]
    Unknown,
}

impl BatteryHealth {
    /// Reads the health out of `/sys/class/power_supply/*/health`.
    pub fn from_sysfs(health: &str) -> Self {
        match health.trim() {
            "Good" => Self::Good,
            "Overheat" | "Hot" => Self::Overheat,
            "Dead" => Self::Dead,
            "Over voltage" => Self::OverVoltage,
            "Unspecified failure" => Self::Failure,
            "Cold" => Self::Cold,
            _ => Self::Unknown,
        }
    }

    /// Reads the health out of Android's `BatteryManager.EXTRA_HEALTH`.
    pub fn from_android(health: i64) -> Self {
        match health {
            2 => Self::Good,
            3 => Self::Overheat,
            4 => Self::Dead,
            5 => Self::OverVoltage,
            6 => Self::Failure,
            7 => Self::Cold,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Overheat => "Overheating",
            Self::Dead => "Dead",
            Self::OverVoltage => "Over voltage",
            Self::Failure => "Failure",
            Self::Cold => "Cold",
            Self::Unknown => "Unknown",
        }
    }
}

/// What the phone is plugged into. `/sys` only says whether the supply is online, so only Android
/// can tell the kinds apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plugged {
    Ac,
    Usb,
    Wireless,
    Dock,
    #[default]
    None,
}

impl Plugged {
    /// Reads the plug out of Android's `BatteryManager.EXTRA_PLUGGED`. Every other non-zero value is
    /// some other dock flag, which the app has always called a dock.
    pub fn from_android(plugged: i64) -> Self {
        match plugged {
            0 => Self::None,
            1 => Self::Ac,
            2 => Self::Usb,
            4 => Self::Wireless,
            _ => Self::Dock,
        }
    }

    /// What the plug is called on screen; nothing at all when the phone is not plugged in.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ac => "AC",
            Self::Usb => "USB",
            Self::Wireless => "Wireless",
            Self::Dock => "Dock",
            Self::None => "",
        }
    }
}

/// One temperature sensor, in degrees Celsius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalZone {
    pub name: String,
    pub celsius: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_mb: u64,
    pub available_mb: u64,
}

impl DiskInfo {
    /// From raw byte counts, for a volume the machine will not name: the phone reports one file
    /// system through Java and says nothing about where it is mounted, the desktop knows both.
    pub fn from_bytes(total: u64, available: u64) -> Self {
        const BYTES_PER_MB: u64 = 1024 * 1024;
        Self {
            name: String::new(),
            mount_point: String::new(),
            file_system: String::new(),
            total_mb: total / BYTES_PER_MB,
            available_mb: available / BYTES_PER_MB,
        }
    }

    pub fn used_mb(&self) -> u64 {
        self.total_mb.saturating_sub(self.available_mb)
    }

    pub fn used_percent(&self) -> f32 {
        percent(self.used_mb(), self.total_mb)
    }
}

/// One network interface: how fast it is moving bytes right now, and how much it has moved since
/// the machine booted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkInfo {
    pub interface: String,
    pub received_bytes_per_second: u64,
    pub transmitted_bytes_per_second: u64,
    pub total_received_bytes: u64,
    pub total_transmitted_bytes: u64,
}

/// The temperature of the first sensor called `name`. The helper names its sensors `cpu`, `gpu`,
/// `battery` and `skin`; the desktop uses whatever labels the kernel writes.
pub fn celsius_of(zones: &[ThermalZone], name: &str) -> Option<f32> {
    zones
        .iter()
        .find(|zone| zone.name == name)
        .map(|zone| zone.celsius)
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
    fn a_disk_can_be_built_from_bytes_when_it_has_no_name() {
        let disk = DiskInfo::from_bytes(500 * 1024 * 1024 * 1024, 100 * 1024 * 1024 * 1024);
        assert_eq!((disk.total_mb, disk.available_mb), (512_000, 102_400));
        assert_eq!(disk.used_mb(), 409_600);
        assert_eq!(disk.used_percent(), 80.0);
        assert!(disk.name.is_empty() && disk.mount_point.is_empty());
    }

    #[test]
    fn a_temperature_is_found_by_the_name_the_producer_uses() {
        let zones = vec![
            ThermalZone { name: "cpu".to_string(), celsius: 46.9 },
            ThermalZone { name: "battery".to_string(), celsius: 37.5 },
        ];
        assert_eq!(celsius_of(&zones, "battery"), Some(37.5));
        assert_eq!(celsius_of(&zones, "skin"), None);
        assert_eq!(celsius_of(&[], "cpu"), None);
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

    #[test]
    fn battery_states_and_health_come_from_android_too() {
        assert_eq!(BatteryState::from_android(2), BatteryState::Charging);
        assert_eq!(BatteryState::from_android(5), BatteryState::Full);
        assert_eq!(BatteryState::from_android(1), BatteryState::Unknown);
        assert_eq!(BatteryHealth::from_android(2), BatteryHealth::Good);
        assert_eq!(BatteryHealth::from_android(6), BatteryHealth::Failure);
        assert_eq!(BatteryHealth::from_android(0), BatteryHealth::Unknown);
    }

    #[test]
    fn battery_health_comes_from_the_kernel_and_reads_back_as_words() {
        assert_eq!(BatteryHealth::from_sysfs("Good"), BatteryHealth::Good);
        assert_eq!(BatteryHealth::from_sysfs("Over voltage"), BatteryHealth::OverVoltage);
        assert_eq!(BatteryHealth::from_sysfs("Unspecified failure"), BatteryHealth::Failure);
        assert_eq!(BatteryHealth::from_sysfs("Warm"), BatteryHealth::Unknown);
        assert_eq!(BatteryHealth::default().label(), "Unknown");
        assert_eq!(BatteryHealth::Overheat.label(), "Overheating");
    }

    #[test]
    fn a_plug_is_named_or_absent() {
        assert_eq!(Plugged::from_android(0), Plugged::None);
        assert_eq!(Plugged::from_android(1), Plugged::Ac);
        assert_eq!(Plugged::from_android(4), Plugged::Wireless);
        assert_eq!(Plugged::from_android(8), Plugged::Dock);
        assert_eq!(Plugged::from_android(99), Plugged::Dock);
        assert_eq!(Plugged::None.label(), "");
        assert_eq!(Plugged::Usb.label(), "USB");
        assert_eq!(BatteryState::Charging.label(), "Charging");
        assert_eq!(BatteryState::NotCharging.label(), "On battery");
    }

    #[test]
    fn power_is_volts_times_amps_and_needs_both() {
        let battery = BatteryInfo {
            percent: 57.0,
            state: BatteryState::Charging,
            voltage_mv: Some(4012),
            current_ma: Some(1500),
            ..Default::default()
        };
        assert!((battery.watts().unwrap() - 6.018).abs() < 0.001);
        assert!(battery.is_charging() && !battery.is_full());
        assert_eq!(BatteryInfo { voltage_mv: None, ..battery }.watts(), None);
    }
}
