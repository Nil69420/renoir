use ::renoir::topic_rings::{FrameChannel, LeanQos};
use pyo3::{prelude::*, types::PyBytes};
use std::time::Duration;

use crate::map_err;

#[pyclass(name = "FrameChannel")]
pub struct PyFrameChannel {
    inner: FrameChannel,
}

#[pymethods]
impl PyFrameChannel {
    #[new]
    #[pyo3(signature = (name, max_frame, slot_count, multi_producer=false, consumer=false, from_oldest=false))]
    fn new(
        name: &str,
        max_frame: usize,
        slot_count: usize,
        multi_producer: bool,
        consumer: bool,
        from_oldest: bool,
    ) -> PyResult<Self> {
        let inner = FrameChannel::open(
            name,
            max_frame,
            slot_count,
            multi_producer,
            consumer,
            from_oldest,
        )
        .map_err(map_err)?;
        Ok(Self { inner })
    }

    #[staticmethod]
    #[pyo3(signature = (name, max_frame, depth=8, keep_all=false, transient_local=false,
        deadline_ms=None, lifespan_ms=None, lease_ms=None, exclusive_strength=None,
        consumer=false, multi_producer=false, require_offer=false))]
    #[allow(clippy::too_many_arguments)]
    fn with_qos(
        name: &str,
        max_frame: usize,
        depth: usize,
        keep_all: bool,
        transient_local: bool,
        deadline_ms: Option<u64>,
        lifespan_ms: Option<u64>,
        lease_ms: Option<u64>,
        exclusive_strength: Option<u64>,
        consumer: bool,
        multi_producer: bool,
        require_offer: bool,
    ) -> PyResult<Self> {
        let mut qos = LeanQos::reliable(depth);
        if keep_all {
            qos = qos.keep_all();
        }
        if transient_local {
            qos = qos.transient_local();
        }
        if require_offer {
            qos = qos.require_offer();
        }
        if let Some(v) = deadline_ms {
            qos = qos.with_deadline(Duration::from_millis(v));
        }
        if let Some(v) = lifespan_ms {
            qos = qos.with_lifespan(Duration::from_millis(v));
        }
        if let Some(v) = lease_ms {
            qos = qos.with_manual_liveliness(Duration::from_millis(v));
        }
        if let Some(v) = exclusive_strength {
            qos = qos.exclusive(v);
        }
        let inner = FrameChannel::open_qos(name, max_frame, qos, multi_producer, consumer)
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

    fn read_qos<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        match self.inner.view_qos().map_err(map_err)? {
            Some(view) => {
                let bytes = PyBytes::new(py, view.bytes());
                view.release();
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    fn deadline_misses(&self) -> u64 {
        self.inner.deadline_misses()
    }

    #[pyo3(signature = (payload, notify=false))]
    fn publish(&mut self, payload: &Bound<'_, PyBytes>, notify: bool) -> PyResult<()> {
        let bytes = payload.as_bytes();
        let mut loan = self.inner.loan(bytes.len()).map_err(map_err)?;
        loan.bytes_mut().copy_from_slice(bytes);
        if notify {
            loan.commit_notify().map_err(map_err)
        } else {
            loan.commit().map_err(map_err)
        }
    }

    #[pyo3(signature = (max_age_ms=None))]
    fn read<'py>(
        &mut self,
        py: Python<'py>,
        max_age_ms: Option<u64>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let view = match max_age_ms {
            Some(ms) => self
                .inner
                .view_fresh(std::time::Duration::from_millis(ms))
                .map_err(map_err)?,
            None => self.inner.view().map_err(map_err)?,
        };
        Ok(view.map(|view| PyBytes::new(py, view.bytes())))
    }

    fn stale_dropped(&self) -> u64 {
        self.inner.stale_dropped()
    }

    #[pyo3(signature = (spin_iterations=2000, timeout_ms=None))]
    fn wait(&mut self, py: Python<'_>, spin_iterations: u32, timeout_ms: Option<u64>) -> bool {
        let timeout = timeout_ms.map(std::time::Duration::from_millis);
        py.detach(|| self.inner.wait(spin_iterations, timeout))
    }

    fn consumer_id(&self) -> Option<usize> {
        self.inner.consumer_id()
    }

    fn set_upstream(&mut self, upstream: usize) -> PyResult<()> {
        self.inner.set_upstream(upstream).map_err(map_err)
    }

    fn live_publishers(&self) -> usize {
        self.inner.live_publishers()
    }

    #[getter]
    fn max_frame(&self) -> usize {
        self.inner.max_frame()
    }
}
