use ::renoir::topic_rings::{
    LeanChannel, LeanClock, LeanMode, LeanQos, LeanRead, LeanWaitStrategy,
};
use pyo3::{prelude::*, types::PyBytes};
use std::time::Duration;

use crate::map_err;

#[pyclass(name = "LeanChannel")]
pub struct PyLeanChannel {
    inner: LeanChannel,
}

#[pymethods]
impl PyLeanChannel {
    #[new]
    #[pyo3(signature = (name, msg_size, slot_count, from_oldest=false, multi_producer=false, gated=false, consumer=false, clock=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        name: &str,
        msg_size: usize,
        slot_count: usize,
        from_oldest: bool,
        multi_producer: bool,
        gated: bool,
        consumer: bool,
        clock: Option<&str>,
    ) -> PyResult<Self> {
        let mode = LeanMode::new(multi_producer, gated);
        let clock = match clock {
            None | Some("none") => LeanClock::None,
            Some("nanoseconds") | Some("ns") => LeanClock::Nanoseconds,
            Some("ticks") => LeanClock::Ticks,
            Some(other) => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "clock must be None, 'nanoseconds' or 'ticks', not '{other}'"
                )))
            }
        };
        let mut inner = if clock.is_stamped() {
            LeanChannel::open_stamped(name, msg_size, slot_count, from_oldest, mode, clock)
        } else {
            LeanChannel::open_with_mode(name, msg_size, slot_count, from_oldest, mode)
        }
        .map_err(map_err)?;
        if consumer {
            inner.register_consumer(from_oldest).map_err(map_err)?;
        }
        Ok(Self { inner })
    }

    #[staticmethod]
    #[pyo3(signature = (name, msg_size, depth=64, reliable=false, keep_all=false, transient_local=false,
        deadline_ms=None, lifespan_ms=None, min_separation_ms=None, lease_ms=None,
        exclusive_strength=None, clock=None, subscriber=false, multi_producer=false))]
    #[allow(clippy::too_many_arguments)]
    fn with_qos(
        name: &str,
        msg_size: usize,
        depth: usize,
        reliable: bool,
        keep_all: bool,
        transient_local: bool,
        deadline_ms: Option<u64>,
        lifespan_ms: Option<u64>,
        min_separation_ms: Option<u64>,
        lease_ms: Option<u64>,
        exclusive_strength: Option<u64>,
        clock: Option<&str>,
        subscriber: bool,
        multi_producer: bool,
    ) -> PyResult<Self> {
        let mut qos = if reliable {
            LeanQos::reliable(depth)
        } else {
            LeanQos::best_effort(depth)
        };
        if keep_all {
            qos = qos.keep_all();
        }
        if transient_local {
            qos = qos.transient_local();
        }
        let clock = match clock {
            None | Some("none") => LeanClock::None,
            Some("nanoseconds") | Some("ns") => LeanClock::Nanoseconds,
            Some("ticks") => LeanClock::Ticks,
            Some(other) => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "clock must be None, 'nanoseconds' or 'ticks', not '{other}'"
                )))
            }
        };
        qos = qos.with_clock(clock);
        if let Some(v) = deadline_ms {
            qos = qos.with_deadline(Duration::from_millis(v));
        }
        if let Some(v) = lifespan_ms {
            qos = qos.with_lifespan(Duration::from_millis(v));
        }
        if let Some(v) = min_separation_ms {
            qos = qos.with_min_separation(Duration::from_millis(v));
        }
        if let Some(v) = lease_ms {
            qos = qos.with_manual_liveliness(Duration::from_millis(v));
        }
        if let Some(v) = exclusive_strength {
            qos = qos.exclusive(v);
        }
        let inner = LeanChannel::open_qos(name, msg_size, qos, multi_producer, subscriber)
            .map_err(map_err)?;
        Ok(Self { inner })
    }

    fn register_publisher(&mut self) -> PyResult<()> {
        self.inner.register_publisher().map_err(map_err)
    }

    fn assert_liveliness(&mut self) -> PyResult<()> {
        self.inner.assert_liveliness().map_err(map_err)
    }

    fn liveliness<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let status = self.inner.liveliness();
        let d = pyo3::types::PyDict::new(py);
        d.set_item("registered", status.registered)?;
        d.set_item("alive", status.alive)?;
        d.set_item("lost", status.lost)?;
        d.set_item("lost_count", status.lost_count)?;
        Ok(d)
    }

    fn is_owner(&mut self) -> bool {
        self.inner.is_owner()
    }

    fn try_read_stamped_qos<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<Option<(u64, Bound<'py, PyBytes>)>> {
        let mut buffer = vec![0u8; self.inner.ring().data_size()];
        match self
            .inner
            .try_read_stamped_qos(&mut buffer)
            .map_err(map_err)?
        {
            Some(stamp) => Ok(Some((stamp, PyBytes::new(py, &buffer)))),
            None => Ok(None),
        }
    }

    fn stale_dropped(&self) -> u64 {
        self.inner.stale_dropped()
    }

    fn filtered_out(&self) -> u64 {
        self.inner.filtered_out()
    }

    fn publish(&mut self, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner.publish(payload.as_bytes()).map_err(map_err)
    }

    fn publish_notify(&mut self, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .publish_notify(payload.as_bytes())
            .map_err(map_err)
    }

    #[pyo3(signature = (spin_iterations=2000, timeout_ms=None))]
    fn wait(&self, py: Python<'_>, spin_iterations: u32, timeout_ms: Option<u64>) -> bool {
        let timeout = timeout_ms.map(std::time::Duration::from_millis);
        py.detach(|| self.inner.wait_readable(spin_iterations, timeout))
    }

    fn publish_batch(&mut self, payload: &Bound<'_, PyBytes>) -> PyResult<usize> {
        self.inner
            .publish_batch(payload.as_bytes())
            .map_err(map_err)
    }

    fn read_batch<'py>(
        &mut self,
        py: Python<'py>,
        max_messages: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let size = self.inner.msg_size();
        let mut result = Ok(0usize);
        let inner = &mut self.inner;
        let bytes = PyBytes::new_with(py, size * max_messages, |buf| {
            result = inner.read_batch(buf);
            Ok(())
        })?;
        let count = result.map_err(map_err)?;
        Ok(PyBytes::new(py, &bytes.as_bytes()[..count * size]))
    }

    #[pyo3(signature = (timeout_ms=None))]
    fn wait_adaptive(&mut self, py: Python<'_>, timeout_ms: Option<u64>) -> bool {
        let timeout = timeout_ms.map(std::time::Duration::from_millis);
        let inner = &mut self.inner;
        py.detach(|| inner.wait_adaptive(timeout))
    }

    #[pyo3(signature = (deadline_ms=None))]
    fn set_deadline(&mut self, deadline_ms: Option<u64>) {
        self.inner
            .set_deadline(deadline_ms.map(std::time::Duration::from_millis));
    }

    #[pyo3(signature = (spin_iterations=2000))]
    fn wait_deadline(&mut self, py: Python<'_>, spin_iterations: u32) -> bool {
        let inner = &mut self.inner;
        py.detach(|| {
            inner.wait_deadline(spin_iterations) == ::renoir::topic_rings::DeadlineWait::Ready
        })
    }

    fn deadline_misses(&self) -> u64 {
        self.inner.deadline_misses()
    }

    fn try_read<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let size = self.inner.msg_size();
        let mut out = None;
        let mut failure = None;
        let inner = &mut self.inner;
        let bytes = PyBytes::new_with(py, size, |buf| match inner.try_read(buf) {
            Ok(LeanRead::Message) => {
                out = Some(());
                Ok(())
            }
            Ok(LeanRead::Empty) => Ok(()),
            Err(e) => {
                failure = Some(e);
                Ok(())
            }
        })?;
        if let Some(e) = failure {
            return Err(map_err(e));
        }
        Ok(out.map(|()| bytes))
    }

    fn skip_to_latest(&mut self, keep: u64) -> u64 {
        self.inner.skip_to_latest(keep)
    }

    fn lost(&self) -> u64 {
        self.inner.lost()
    }

    fn pending(&self) -> usize {
        self.inner.pending()
    }

    fn live_publishers(&self) -> usize {
        self.inner.live_publishers()
    }

    fn consumer_id(&self) -> Option<usize> {
        self.inner.consumer_id()
    }

    fn set_upstream(&mut self, upstream: usize) -> PyResult<()> {
        self.inner.set_upstream(upstream).map_err(map_err)
    }

    #[getter]
    fn msg_size(&self) -> usize {
        self.inner.ring().data_size()
    }

    #[getter]
    fn clock(&self) -> &'static str {
        match self.inner.ring().clock() {
            LeanClock::None => "none",
            LeanClock::Nanoseconds => "nanoseconds",
            LeanClock::Ticks => "ticks",
        }
    }

    #[getter]
    fn tick_hz(&self) -> u64 {
        self.inner.ring().tick_hz()
    }

    fn stamp_now(&self) -> u64 {
        self.inner.ring().stamp_now()
    }

    fn stamp_to_ns(&self, stamp: u64) -> u64 {
        self.inner.ring().ticks_to_ns(stamp)
    }

    fn publish_at(&mut self, stamp: u64, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .publish_at(stamp, payload.as_bytes())
            .map_err(map_err)
    }

    fn publish_now(&mut self, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner.publish_now(payload.as_bytes()).map_err(map_err)
    }

    fn publish_at_notify(&mut self, stamp: u64, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .publish_at_notify(stamp, payload.as_bytes())
            .map_err(map_err)
    }

    fn publish_now_notify(&mut self, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .publish_now_notify(payload.as_bytes())
            .map_err(map_err)
    }

    fn try_read_stamped<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<Option<(u64, Bound<'py, PyBytes>)>> {
        let size = self.inner.ring().data_size();
        let mut buffer = vec![0u8; size];
        match self.inner.try_read_stamped(&mut buffer).map_err(map_err)? {
            Some(stamp) => Ok(Some((stamp, PyBytes::new(py, &buffer)))),
            None => Ok(None),
        }
    }

    fn try_read_lagged<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<(Option<Bound<'py, PyBytes>>, u64)> {
        let mut buffer = vec![0u8; self.inner.msg_size()];
        let (read, missed) = self.inner.try_read_lagged(&mut buffer).map_err(map_err)?;
        let message = matches!(read, LeanRead::Message).then(|| PyBytes::new(py, &buffer));
        Ok((message, missed))
    }

    #[pyo3(signature = (strategy, timeout_ms=None))]
    fn wait_with(&self, py: Python<'_>, strategy: &str, timeout_ms: Option<u64>) -> PyResult<bool> {
        let strategy = match strategy {
            "busy_spin" => LeanWaitStrategy::BusySpin,
            "yielding" => LeanWaitStrategy::Yielding,
            "sleeping" => LeanWaitStrategy::Sleeping,
            "blocking" => LeanWaitStrategy::Blocking,
            "phased_backoff" => LeanWaitStrategy::PhasedBackoff,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown wait strategy '{other}'"
                )))
            }
        };
        let timeout = timeout_ms.map(Duration::from_millis);
        Ok(py.detach(|| self.inner.wait_with(strategy, timeout)))
    }

    #[pyo3(signature = (spin_us, yield_us, timeout_ms=None))]
    fn wait_phased(
        &self,
        py: Python<'_>,
        spin_us: u64,
        yield_us: u64,
        timeout_ms: Option<u64>,
    ) -> bool {
        let timeout = timeout_ms.map(Duration::from_millis);
        py.detach(|| {
            self.inner.wait_phased(
                Duration::from_micros(spin_us),
                Duration::from_micros(yield_us),
                timeout,
            )
        })
    }
}

#[pyfunction]
pub fn lean_memory_required(msg_size: usize, slot_count: usize) -> PyResult<usize> {
    let slot_bytes = ::renoir::topic_rings::lean_slot_bytes(msg_size)
        .ok_or_else(|| crate::RenoirError::new_err("msg_size must be between 1 and 504"))?;
    if slot_count < 2 {
        return Err(crate::RenoirError::new_err("slot_count must be at least 2"));
    }
    let ring_bytes = slot_bytes * slot_count;
    let slots = ::renoir::topic_rings::LeanRing::slot_count_for(ring_bytes, msg_size);
    Ok(::renoir::topic_rings::LEAN_HEADER_BYTES + slots * slot_bytes)
}
