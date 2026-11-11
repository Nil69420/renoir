mod roles;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STALL: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
const RSS_SLACK_KB: u64 = 20_000;
const FD_SLACK: usize = 2;
const SHM_SLACK: usize = 3;

#[derive(Clone, Copy)]
struct Spec {
    role: &'static str,
    topic: &'static str,
    id: u64,
}

const SPECS: [Spec; 11] = [
    Spec {
        role: "prod",
        topic: "soak_be",
        id: 1,
    },
    Spec {
        role: "prod",
        topic: "soak_be",
        id: 2,
    },
    Spec {
        role: "cons",
        topic: "soak_be",
        id: 1,
    },
    Spec {
        role: "prod",
        topic: "soak_rel",
        id: 1,
    },
    Spec {
        role: "cons",
        topic: "soak_rel",
        id: 1,
    },
    Spec {
        role: "cons",
        topic: "soak_rel",
        id: 2,
    },
    Spec {
        role: "prod",
        topic: "soak_own",
        id: 1,
    },
    Spec {
        role: "prod",
        topic: "soak_own",
        id: 2,
    },
    Spec {
        role: "cons",
        topic: "soak_own",
        id: 1,
    },
    Spec {
        role: "rpcs",
        topic: "",
        id: 0,
    },
    Spec {
        role: "rpcc",
        topic: "",
        id: 0,
    },
];

struct Slot {
    spec: Spec,
    child: Child,
    started: Instant,
    count: Arc<AtomicU64>,
    changed_ms: Arc<AtomicU64>,
    base: Option<(u64, usize)>,
    killed: u32,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn spawn(spec: Spec, epoch: Instant) -> Slot {
    let mut child = Command::new(std::env::current_exe().expect("exe"))
        .args([spec.role, spec.topic, &spec.id.to_string()])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let out = child.stdout.take().expect("stdout");
    let count = Arc::new(AtomicU64::new(0));
    let changed_ms = Arc::new(AtomicU64::new(epoch.elapsed().as_millis() as u64));
    let (c, m) = (count.clone(), changed_ms.clone());
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(n) = line.rsplit(' ').next().and_then(|v| v.parse::<u64>().ok()) {
                if n != c.swap(n, Ordering::Relaxed) {
                    m.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                }
            }
        }
    });
    Slot {
        spec,
        child,
        started: Instant::now(),
        count,
        changed_ms,
        base: None,
        killed: 0,
    }
}

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn fd_count(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd")).map_or(0, Iterator::count)
}

fn shm_stats(dir: &str) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata() {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

fn child_main(args: &[String]) {
    let id: u64 = args[2].parse().unwrap_or(0);
    match args[0].as_str() {
        "prod" => roles::producer(&args[1], id),
        "cons" => roles::consumer(&args[1], id),
        "rpcs" => roles::rpc_server(),
        _ => roles::rpc_client(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 3 {
        child_main(&args);
        return;
    }
    let secs: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(60);
    let kill_every: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(8);
    let provided = std::env::var("RENOIR_SHM_DIR").ok();
    let dir = provided
        .clone()
        .unwrap_or_else(|| format!("/dev/shm/renoir_chaos_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("shm dir");
    std::env::set_var("RENOIR_SHM_DIR", &dir);

    let epoch = Instant::now();
    let end = epoch + Duration::from_secs(secs);
    let mut rng = Rng(epoch.elapsed().as_nanos() as u64 | 0x9E37_79B9_7F4A_7C15 | 1);
    let mut slots: Vec<Slot> = SPECS.iter().map(|s| spawn(*s, epoch)).collect();
    let mut next_kill = Instant::now() + Duration::from_secs(5);
    let mut next_sample = Instant::now() + Duration::from_secs(30);
    let mut shm_base: Option<(usize, u64)> = None;
    let (mut kills, mut failures) = (0u32, Vec::<String>::new());

    while Instant::now() < end && failures.is_empty() {
        std::thread::sleep(Duration::from_millis(200));
        let now_ms = epoch.elapsed().as_millis() as u64;
        for s in &mut slots {
            if let Ok(Some(status)) = s.child.try_wait() {
                failures.push(format!(
                    "{}:{}:{} exited on its own: {status}",
                    s.spec.role, s.spec.topic, s.spec.id
                ));
            } else if s.started.elapsed() > STALL
                && now_ms.saturating_sub(s.changed_ms.load(Ordering::Relaxed))
                    > STALL.as_millis() as u64
            {
                failures.push(format!(
                    "{}:{}:{} stalled at count {}",
                    s.spec.role,
                    s.spec.topic,
                    s.spec.id,
                    s.count.load(Ordering::Relaxed)
                ));
            }
        }
        if Instant::now() >= next_kill {
            let i = (rng.next() % slots.len() as u64) as usize;
            let s = &mut slots[i];
            let _ = s.child.kill();
            let _ = s.child.wait();
            std::thread::sleep(Duration::from_millis(rng.next() % 200));
            let killed = s.killed + 1;
            *s = spawn(s.spec, epoch);
            s.killed = killed;
            kills += 1;
            next_kill = Instant::now() + Duration::from_secs(1 + rng.next() % (2 * kill_every));
        }
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs(30);
            let (files, bytes) = shm_stats(&dir);
            let base = *shm_base.get_or_insert((files, bytes));
            println!(
                "t={:>6}s kills={kills} shm_files={files} shm_bytes={bytes}",
                epoch.elapsed().as_secs()
            );
            if files > base.0 + SHM_SLACK {
                failures.push(format!("shm files grew from {} to {files}", base.0));
            }
            for s in &mut slots {
                let pid = s.child.id();
                let (rss, fds) = (rss_kb(pid), fd_count(pid));
                println!(
                    "  {}:{}:{} pid={pid} count={} rss_kb={rss} fds={fds} restarts={}",
                    s.spec.role,
                    s.spec.topic,
                    s.spec.id,
                    s.count.load(Ordering::Relaxed),
                    s.killed
                );
                if s.started.elapsed() > SETTLE {
                    let base = *s.base.get_or_insert((rss, fds));
                    if rss > base.0 + RSS_SLACK_KB || fds > base.1 + FD_SLACK {
                        failures.push(format!(
                            "{}:{}:{} grew: rss {} -> {rss} kB, fds {} -> {fds}",
                            s.spec.role, s.spec.topic, s.spec.id, base.0, base.1
                        ));
                    }
                }
            }
        }
    }

    for s in &mut slots {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
    if provided.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!(
        "duration={}s kills={kills} failures={}",
        epoch.elapsed().as_secs(),
        failures.len()
    );
    for f in &failures {
        println!("FAIL {f}");
    }
    println!(
        "{}",
        if failures.is_empty() {
            "SOAK PASS"
        } else {
            "SOAK FAIL"
        }
    );
    std::process::exit(i32::from(!failures.is_empty()));
}
