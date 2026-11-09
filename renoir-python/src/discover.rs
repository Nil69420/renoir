use pyo3::prelude::*;
use pyo3::types::PyDict;

use ::renoir::discovery::{scan, sweep as sweep_rings, RingKind, SweepOptions};
use ::renoir::topic_rings::{LeanClock, LeanMode, STAMP_BYTES};

#[pyfunction]
pub fn discover(py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
    let rings = py.detach(scan);
    let mut out = Vec::with_capacity(rings.len());
    for ring in rings {
        let d = PyDict::new(py);
        d.set_item("name", &ring.name)?;
        d.set_item(
            "kind",
            match ring.kind {
                RingKind::Lean => "lean",
                RingKind::Broadcast => "broadcast",
                RingKind::Spsc => "spsc",
                RingKind::Mpmc => "mpmc",
            },
        )?;
        d.set_item("file_bytes", ring.file_bytes)?;
        d.set_item("mapped_by", &ring.mapped_by)?;
        if let Some(l) = &ring.lean {
            d.set_item(
                "mode",
                match l.mode {
                    LeanMode::SingleProducer => "single_producer",
                    LeanMode::MultiProducer => "multi_producer",
                    LeanMode::SingleProducerGated => "single_producer_gated",
                    LeanMode::MultiProducerGated => "multi_producer_gated",
                },
            )?;
            d.set_item(
                "clock",
                match l.clock {
                    LeanClock::None => "none",
                    LeanClock::Nanoseconds => "nanoseconds",
                    LeanClock::Ticks => "ticks",
                },
            )?;
            d.set_item(
                "msg_size",
                if l.clock.is_stamped() {
                    l.msg_size.saturating_sub(STAMP_BYTES as u64)
                } else {
                    l.msg_size
                },
            )?;
            d.set_item("slot_count", l.slot_count)?;
            d.set_item("published", l.published)?;
            d.set_item("live_publishers", l.live_publishers)?;
            d.set_item("registered_consumers", l.registered_consumers)?;
        }
        out.push(d.unbind());
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (dry_run=false, ignore_unreadable=false))]
pub fn sweep(py: Python<'_>, dry_run: bool, ignore_unreadable: bool) -> usize {
    py.detach(|| {
        sweep_rings(SweepOptions {
            dry_run,
            ignore_unreadable_processes: ignore_unreadable,
        })
        .removed
        .len()
    })
}
