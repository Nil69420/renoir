use ::renoir::topic_rings::{LeanChannel, LeanMode, LeanRead};
use pyo3::{prelude::*, types::PyBytes};

use crate::map_err;

#[pyclass(name = "LeanChannel")]
pub struct PyLeanChannel {
    inner: LeanChannel,
}

#[pymethods]
impl PyLeanChannel {
    #[new]
    #[pyo3(signature = (name, msg_size, slot_count, from_oldest=false, multi_producer=false, gated=false, consumer=false))]
    fn new(
        name: &str,
        msg_size: usize,
        slot_count: usize,
        from_oldest: bool,
        multi_producer: bool,
        gated: bool,
        consumer: bool,
    ) -> PyResult<Self> {
        let mode = LeanMode::new(multi_producer, gated);
        let mut inner = LeanChannel::open_with_mode(name, msg_size, slot_count, from_oldest, mode)
            .map_err(map_err)?;
        if consumer {
            inner.register_consumer(from_oldest).map_err(map_err)?;
        }
        Ok(Self { inner })
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

    #[getter]
    fn msg_size(&self) -> usize {
        self.inner.msg_size()
    }
}
