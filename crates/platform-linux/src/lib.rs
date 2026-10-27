//! The Linux backend: reads `/proc` and `/sys` through `sysinfo` and turns them into the domain
//! types. Everything operating-system-specific sits here, so `domain` stays clean and the code that
//! displays a snapshot does not care where the numbers came from.

use std::thread;

use domain::{CpuInfo, MemoryInfo, Process, Snapshot};
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, System};

const BYTES_PER_MB: u64 = 1024 * 1024;

/// Takes one reading of the machine.
pub fn snapshot() -> Snapshot {
    let mut sys = System::new_all();

    // CPU percentages are the difference between two samples. `new_all` took the first one, so wait
    // the shortest interval `sysinfo` considers meaningful and take the second; without this every
    // number would read 0%.
    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();

    Snapshot {
        cpu: cpu_info(&sys),
        memory: memory_info(&sys),
        processes: processes(&sys),
    }
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
