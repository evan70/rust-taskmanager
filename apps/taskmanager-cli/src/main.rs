//! Prints one reading of the machine: CPU, memory, battery, temperatures, disks, network and the
//! ten busiest processes. `--json` prints the same reading as JSON, which is what the future UI and
//! the tests will read.
//!
//! Nothing here panics: a machine without a battery or without sensors prints less, and a reading
//! that could not be taken at all ends in an error message and a non-zero exit code.

use std::process::ExitCode;

use domain::{BatteryInfo, DiskInfo, NetworkInfo, Snapshot};
use platform_linux::snapshot;

const TOP_PROCESSES: usize = 10;

fn main() -> ExitCode {
    let json = std::env::args_os().any(|argument| argument == "--json");

    let snapshot = match snapshot() {
        Ok(snapshot) => snapshot,
        Err(err) => {
            eprintln!("taskmanager-cli: {err}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        return print_json(&snapshot);
    }

    print_text(&snapshot);
    ExitCode::SUCCESS
}

fn print_json(snapshot: &Snapshot) -> ExitCode {
    match serde_json::to_string_pretty(snapshot) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("taskmanager-cli: cannot write JSON: {err}");
            ExitCode::FAILURE
        }
    }
}

fn print_text(snapshot: &Snapshot) {
    println!("=== CPU ===");
    println!("Total: {:.1}%", snapshot.cpu.total_percent);
    for (core, percent) in snapshot.cpu.cores.iter().enumerate() {
        println!("  Core {core}: {percent:.1}%");
    }
    let [one, five, fifteen] = snapshot.cpu.load_avg;
    println!("Load: {one:.2} {five:.2} {fifteen:.2}");

    println!();
    println!("=== Memory ===");
    println!(
        "Used: {} MB / {} MB ({:.1}%)",
        snapshot.memory.used_mb(),
        snapshot.memory.total,
        snapshot.memory.used_percent()
    );
    if snapshot.memory.swap_total > 0 {
        println!(
            "Swap in use: {} MB of {} MB",
            snapshot.memory.swap_used_mb(),
            snapshot.memory.swap_total
        );
    }

    println!();
    println!("=== Battery ===");
    match &snapshot.battery {
        Some(battery) => print_battery(battery),
        None => println!("No battery on this machine."),
    }

    println!();
    println!("=== Temperatures ===");
    if snapshot.thermals.is_empty() {
        println!("No sensor readable (needs /sys/class/hwmon).");
    }
    for zone in &snapshot.thermals {
        println!("  {:<28} {:>6.1} °C", zone.name, zone.celsius);
    }

    println!();
    println!("=== Disks ===");
    if snapshot.disks.is_empty() {
        println!("No mounted file system reports a size.");
    }
    for disk in &snapshot.disks {
        print_disk(disk);
    }

    println!();
    println!("=== Network ===");
    if snapshot.networks.is_empty() {
        println!("No interface.");
    }
    for network in &snapshot.networks {
        print_network(network);
    }

    println!();
    println!("=== Top {TOP_PROCESSES} processes by CPU ===");
    // `total_cmp` instead of `partial_cmp().unwrap()`: a process that reports a NaN percentage
    // should sort last, not panic the program.
    let mut processes = snapshot.processes.clone();
    processes.sort_by(|left, right| right.cpu_percent.total_cmp(&left.cpu_percent));
    for process in processes.iter().take(TOP_PROCESSES) {
        println!(
            "  {:>6}  {:>5.1}%  {:>6} MB  {}",
            process.pid, process.cpu_percent, process.memory_mb, process.name
        );
    }
}

fn print_battery(battery: &BatteryInfo) {
    println!("{:.0}% ({:?})", battery.percent, battery.state);
}

fn print_disk(disk: &DiskInfo) {
    println!(
        "  {:<24} {:<16} {:>10} used of {:>10}  ({:.1}%)",
        disk.mount_point,
        disk.file_system,
        size(disk.used_mb()),
        size(disk.total_mb),
        disk.used_percent()
    );
}

fn print_network(network: &NetworkInfo) {
    println!(
        "  {:<16} {:>10} down {:>10} up   since boot: {:>10} down {:>10} up",
        network.interface,
        speed(network.received_bytes_per_second),
        speed(network.transmitted_bytes_per_second),
        bytes(network.total_received_bytes),
        bytes(network.total_transmitted_bytes),
    );
}

/// A speed, in the unit people read speeds in.
fn speed(bytes_per_second: u64) -> String {
    let kilobytes = bytes_per_second as f64 / 1024.0;
    if kilobytes >= 1024.0 {
        format!("{:.1} MB/s", kilobytes / 1024.0)
    } else {
        format!("{kilobytes:.1} KB/s")
    }
}

fn bytes(bytes: u64) -> String {
    size(bytes / (1024 * 1024))
}

/// Megabytes, printed as gigabytes once they get big enough to be hard to read.
fn size(megabytes: u64) -> String {
    if megabytes >= 1024 {
        format!("{:.1} GB", megabytes as f64 / 1024.0)
    } else {
        format!("{megabytes} MB")
    }
}
