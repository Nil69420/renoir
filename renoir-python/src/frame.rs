use ::renoir::topic_rings::FrameChannel;
use pyo3::{prelude::*, types::PyBytes};

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
