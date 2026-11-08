//! Python bindings for Renoir.
//!
//! A thin PyO3 translation layer over the existing Rust API — no logic is
//! reimplemented here; every call forwards to `::renoir::*`.

mod lean;
mod rpc;

use ::renoir::{
    topic::{TopicConfig, TopicPattern},
    topic_manager_modules::TopicManager,
};
use pyo3::{create_exception, exceptions::PyException, prelude::*, types::PyBytes, Py};

create_exception!(renoir, RenoirError, PyException);

pub(crate) fn map_err(e: ::renoir::RenoirError) -> PyErr {
    PyErr::new::<RenoirError, _>(e.to_string())
}

fn pattern_from_str(pattern: &str) -> Result<TopicPattern, PyErr> {
    match pattern.to_ascii_lowercase().as_str() {
        "spsc" => Ok(TopicPattern::SPSC),
        "spmc" => Ok(TopicPattern::SPMC),
        "mpsc" => Ok(TopicPattern::MPSC),
        "mpmc" => Ok(TopicPattern::MPMC),
        other => Err(PyErr::new::<RenoirError, _>(format!(
            "unknown topic pattern '{}' (expected spsc/spmc/mpsc/mpmc)",
            other
        ))),
    }
}

/// Topic manager wrapper (owns a `TopicManager`)
#[pyclass]
struct PyTopicManager {
    inner: TopicManager,
}

/// Publisher wrapper (owns a `Publisher`)
#[pyclass]
struct PyPublisher {
    inner: ::renoir::topic_manager_modules::Publisher,
}

/// Subscriber wrapper (owns a `Subscriber`)
#[pyclass]
struct PySubscriber {
    inner: ::renoir::topic_manager_modules::Subscriber,
}

#[pymethods]
impl PyTopicManager {
    /// Create a new topic manager
    #[new]
    fn new() -> PyResult<Self> {
        let inner = TopicManager::new().map_err(map_err)?;
        Ok(Self { inner })
    }

    /// Register a topic. `pattern` is one of spsc/spmc/mpsc/mpmc.
    fn create_topic(
        &mut self,
        name: &str,
        pattern: &str,
        ring_capacity: usize,
        max_payload_size: usize,
    ) -> PyResult<u32> {
        let config = TopicConfig {
            name: name.to_string(),
            pattern: pattern_from_str(pattern)?,
            ring_capacity,
            max_payload_size,
            ..Default::default()
        };
        let id = self.inner.create_topic(config).map_err(map_err)?;
        Ok(id)
    }

    fn create_publisher(&mut self, topic_name: &str) -> PyResult<PyPublisher> {
        let inner = self.inner.create_publisher(topic_name).map_err(map_err)?;
        Ok(PyPublisher { inner })
    }

    fn create_subscriber(&mut self, topic_name: &str) -> PyResult<PySubscriber> {
        let inner = self.inner.create_subscriber(topic_name).map_err(map_err)?;
        Ok(PySubscriber { inner })
    }

    fn remove_topic(&mut self, topic_name: &str) -> PyResult<()> {
        self.inner.remove_topic(topic_name).map_err(map_err)
    }
}

#[pymethods]
impl PyPublisher {
    /// Publish a bytes payload; raises `RenoirError` on failure
    fn publish(&self, payload: &Bound<'_, PyBytes>) -> PyResult<()> {
        let data = payload.as_bytes().to_vec();
        self.inner.publish(data).map_err(map_err)
    }
}

#[pymethods]
impl PySubscriber {
    /// Read the next message (blocking up to timeout_ms). Returns bytes or
    /// None on timeout/empty.
    fn read_next(
        &self,
        py: Python<'_>,
        timeout_ms: u64,
    ) -> PyResult<Option<Py<pyo3::types::PyAny>>> {
        let message = if timeout_ms > 0 {
            self.inner
                .wait_for_message(Some(std::time::Duration::from_millis(timeout_ms)))
                .map_err(map_err)?
        } else {
            self.inner.try_subscribe().map_err(map_err)?
        };

        match message {
            Some(msg) => match msg.payload {
                ::renoir::topic::MessagePayload::Inline(data) => {
                    Ok(Some(PyBytes::new(py, &data).into()))
                }
                ::renoir::topic::MessagePayload::Descriptor(_) => {
                    Err(PyErr::new::<RenoirError, _>(
                        "received descriptor message; shared-pool payloads not \
                     supported in Python bindings yet",
                    ))
                }
            },
            None => Ok(None),
        }
    }
}

#[pymodule]
fn renoir_python(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Expose the wrapper classes under their public Python names
    m.add("TopicManager", m.py().get_type::<PyTopicManager>())?;
    m.add("Publisher", m.py().get_type::<PyPublisher>())?;
    m.add("Subscriber", m.py().get_type::<PySubscriber>())?;
    m.add("LeanChannel", m.py().get_type::<lean::PyLeanChannel>())?;
    m.add_function(wrap_pyfunction!(lean::lean_memory_required, m)?)?;
    m.add("RpcClient", m.py().get_type::<rpc::PyRpcClient>())?;
    m.add("RpcServer", m.py().get_type::<rpc::PyRpcServer>())?;
    m.add("RenoirError", m.py().get_type::<RenoirError>())?;
    Ok(())
}
