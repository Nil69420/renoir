use pyo3::prelude::*;
use pyo3::types::PyDict;

use ::renoir::discovery::{
    scan, sweep as sweep_rings, DiscoveryEvent, DiscoveryWatcher, RingInfo, RingKind, SweepOptions,
};
use ::renoir::topic_rings::{LeanClock, LeanMode, STAMP_BYTES};

fn ring_to_dict<'py>(py: Python<'py>, ring: &RingInfo) -> PyResult<Bound<'py, PyDict>> {
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
    Ok(d)
}

#[pyfunction]
pub fn discover(py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
    let rings = py.detach(scan);
    let mut out = Vec::with_capacity(rings.len());
    for ring in &rings {
        out.push(ring_to_dict(py, ring)?.unbind());
    }
    Ok(out)
}

#[pyclass(name = "DiscoveryWatch")]
pub struct PyDiscoveryWatch {
    inner: DiscoveryWatcher,
}

#[pymethods]
impl PyDiscoveryWatch {
    #[new]
    #[pyo3(signature = (interval_ms=100))]
    fn new(interval_ms: u64) -> PyResult<Self> {
        let inner = DiscoveryWatcher::open(std::time::Duration::from_millis(interval_ms.max(1)))
            .map_err(crate::map_err)?;
        Ok(Self { inner })
    }

    fn fileno(&self) -> i32 {
        self.inner.fd()
    }

    fn snapshot(&self, py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
        let mut out = Vec::new();
        for ring in self.inner.snapshot() {
            out.push(ring_to_dict(py, &ring)?.unbind());
        }
        Ok(out)
    }

    #[pyo3(signature = (timeout_ms=0))]
    fn poll(&mut self, py: Python<'_>, timeout_ms: u64) -> PyResult<Vec<Py<PyDict>>> {
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let inner = &mut self.inner;
        let events = py.detach(|| inner.poll(timeout));
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            let d = PyDict::new(py);
            match event {
                DiscoveryEvent::Appeared(ring) => {
                    d.set_item("event", "appeared")?;
                    d.set_item("ring", ring_to_dict(py, &ring)?)?;
                }
                DiscoveryEvent::Disappeared { name } => {
                    d.set_item("event", "disappeared")?;
                    d.set_item("name", name)?;
                }
                DiscoveryEvent::Changed { previous, current } => {
                    d.set_item("event", "changed")?;
                    d.set_item("previous", ring_to_dict(py, &previous)?)?;
                    d.set_item("ring", ring_to_dict(py, &current)?)?;
                }
            }
            out.push(d.unbind());
        }
        Ok(out)
    }
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
