use std::collections::HashMap;
use std::fmt::Write;
use std::time::Instant;

use super::scan::{RingInfo, RingKind};
use crate::topic_rings::{LeanClock, LeanMode, STAMP_BYTES};

fn kind_name(kind: RingKind) -> &'static str {
    match kind {
        RingKind::Lean => "lean",
        RingKind::Broadcast => "broadcast",
        RingKind::Spsc => "spsc",
        RingKind::Mpmc => "mpmc",
    }
}

fn clock_suffix(clock: LeanClock) -> &'static str {
    match clock {
        LeanClock::None => "",
        LeanClock::Nanoseconds => "+ns",
        LeanClock::Ticks => "+ticks",
    }
}

fn mode_name(mode: LeanMode) -> &'static str {
    match mode {
        LeanMode::SingleProducer => "sp",
        LeanMode::MultiProducer => "mp",
        LeanMode::SingleProducerGated => "sp-gated",
        LeanMode::MultiProducerGated => "mp-gated",
    }
}

pub fn render_table(
    rings: &[RingInfo],
    previous: &HashMap<String, (u64, Instant)>,
    now: Instant,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<28} {:<9} {:<9} {:>7} {:>6} {:>10} {:>5} {:>5} {:>9}  PIDS",
        "NAME", "KIND", "MODE", "MSG", "SLOTS", "PUBLISHED", "PUBS", "CONS", "RATE/s"
    );
    for ring in rings {
        let pids = ring
            .mapped_by
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let dash = String::from("-");
        let (mode, msg, slots, published, pubs, cons, rate) = match &ring.lean {
            Some(l) => (
                format!("{}{}", mode_name(l.mode), clock_suffix(l.clock)),
                if l.clock.is_stamped() {
                    l.msg_size.saturating_sub(STAMP_BYTES as u64).to_string()
                } else {
                    l.msg_size.to_string()
                },
                l.slot_count.to_string(),
                l.published.to_string(),
                l.live_publishers.to_string(),
                l.registered_consumers.to_string(),
                previous.get(&ring.name).map_or(dash.clone(), |(p, t)| {
                    let dt = now.duration_since(*t).as_secs_f64().max(1e-9);
                    format!("{:.0}", l.published.saturating_sub(*p) as f64 / dt)
                }),
            ),
            None => (
                dash.clone(),
                dash.clone(),
                dash.clone(),
                dash.clone(),
                dash.clone(),
                dash.clone(),
                dash,
            ),
        };
        let _ = writeln!(
            out,
            "{:<28} {:<9} {:<9} {:>7} {:>6} {:>10} {:>5} {:>5} {:>9}  {}",
            ring.name,
            kind_name(ring.kind),
            mode,
            msg,
            slots,
            published,
            pubs,
            cons,
            rate,
            pids
        );
    }
    out
}
