//! Prints one reading of the machine: CPU, memory and the ten busiest processes. The smallest
//! possible front end for `platform-linux`, and the place where the domain types get exercised
//! without a phone or a window.

use platform_linux::snapshot;

fn main() {
    let snapshot = snapshot();

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
        snapshot.memory.used_mb,
        snapshot.memory.total_mb,
        snapshot.memory.used_percent()
    );

    println!();
    println!("=== Top 10 processes by CPU ===");
    // `total_cmp` instead of `partial_cmp().unwrap()`: a process that reports a NaN percentage
    // should sort last, not panic the program.
    let mut processes = snapshot.processes;
    processes.sort_by(|left, right| right.cpu_percent.total_cmp(&left.cpu_percent));
    for process in processes.iter().take(10) {
        println!(
            "  {:>6}  {:>5.1}%  {:>6} MB  {}",
            process.pid, process.cpu_percent, process.memory_mb, process.name
        );
    }
}
