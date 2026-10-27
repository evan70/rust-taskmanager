//! The domain types: what one reading of the machine looks like. No I/O, no GUI, no operating
//! system in sight — `platform-linux` fills these in, and whatever displays them (the CLI today,
//! the Android app's screens later) only has to depend on this crate.

use serde::{Deserialize, Serialize};

/// One reading of the machine at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub processes: Vec<Process>,
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
        if self.total_mb == 0 {
            return 0.0;
        }
        (self.used_mb as f32 / self.total_mb as f32) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_mb: u64,
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

    #[test]
    fn used_percent_is_the_share_of_the_total() {
        assert_eq!(memory(8000, 2000).used_percent(), 25.0);
        assert_eq!(memory(8000, 8000).used_percent(), 100.0);
    }

    #[test]
    fn used_percent_survives_a_machine_that_reports_no_memory() {
        assert_eq!(memory(0, 0).used_percent(), 0.0);
    }
}
