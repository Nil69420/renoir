use std::collections::HashMap;
use std::time::{Duration, Instant};

use renoir::discovery::{render_table, scan, sweep, SweepOptions};

fn usage() {
    println!(
        "renoir-monitor [list | watch [SECONDS] | sweep [--dry-run] [--ignore-unreadable]]\n\
         list   one-shot table of rings in the shared-memory directory\n\
         watch  refresh the table every SECONDS (default 1) with message rates\n\
         sweep  delete ring files that no process has mapped"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("list") => print!("{}", render_table(&scan(), &HashMap::new(), Instant::now())),
        Some("watch") => {
            let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let mut previous: HashMap<String, (u64, Instant)> = HashMap::new();
            loop {
                let rings = scan();
                let now = Instant::now();
                print!("\x1b[2J\x1b[H");
                print!("{}", render_table(&rings, &previous, now));
                previous = rings
                    .iter()
                    .filter_map(|r| {
                        r.lean
                            .as_ref()
                            .map(|l| (r.name.clone(), (l.published, now)))
                    })
                    .collect();
                std::thread::sleep(Duration::from_secs_f64(secs.max(0.1)));
            }
        }
        Some("sweep") => {
            let report = sweep(SweepOptions {
                dry_run: args.iter().any(|a| a == "--dry-run"),
                ignore_unreadable_processes: args.iter().any(|a| a == "--ignore-unreadable"),
            });
            for path in &report.removed {
                println!("removed {}", path.display());
            }
            for path in &report.kept_in_use {
                println!("in use  {}", path.display());
            }
            for path in &report.kept_unverifiable {
                println!("kept    {} (a process with unreadable maps could hold it; rerun with --ignore-unreadable to override)", path.display());
            }
        }
        _ => usage(),
    }
}
