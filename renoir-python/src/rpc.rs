use std::time::Duration;

use ::renoir::rpc::{RpcClient, RpcServer};
use pyo3::{prelude::*, types::PyBytes};

use crate::map_err;

#[pyclass(name = "RpcClient")]
pub struct PyRpcClient {
    inner: RpcClient,
    max_payload: usize,
}

#[pymethods]
impl PyRpcClient {
    #[new]
    #[pyo3(signature = (service, max_payload, request_slots=64))]
    fn new(service: &str, max_payload: usize, request_slots: usize) -> PyResult<Self> {
        let inner = RpcClient::open(service, max_payload, request_slots).map_err(map_err)?;
        Ok(Self { inner, max_payload })
    }

    #[pyo3(signature = (request, timeout_ms=1000))]
    fn call<'py>(
        &mut self,
        py: Python<'py>,
        request: &Bound<'_, PyBytes>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut response = vec![0u8; self.max_payload];
        let request = request.as_bytes();
        let inner = &mut self.inner;
        let n = py
            .detach(|| inner.call(request, &mut response, Duration::from_millis(timeout_ms)))
            .map_err(map_err)?;
        Ok(PyBytes::new(py, &response[..n]))
    }
}

#[pyclass(name = "RpcServer")]
pub struct PyRpcServer {
    inner: RpcServer,
    max_payload: usize,
}

#[pymethods]
impl PyRpcServer {
    #[new]
    #[pyo3(signature = (service, max_payload, request_slots=64))]
    fn new(service: &str, max_payload: usize, request_slots: usize) -> PyResult<Self> {
        let inner = RpcServer::open(service, max_payload, request_slots).map_err(map_err)?;
        Ok(Self { inner, max_payload })
    }

    fn poll<'py>(&mut self, py: Python<'py>) -> PyResult<Option<(u32, u64, Bound<'py, PyBytes>)>> {
        let mut buf = vec![0u8; self.max_payload];
        Ok(self.inner.poll(&mut buf).map_err(map_err)?.map(|req| {
            (
                req.client_id,
                req.sequence,
                PyBytes::new(py, &buf[..req.len]),
            )
        }))
    }

    #[pyo3(signature = (timeout_ms=None))]
    fn wait(&self, py: Python<'_>, timeout_ms: Option<u64>) -> bool {
        let timeout = timeout_ms.map(Duration::from_millis);
        py.detach(|| self.inner.wait_request(2000, timeout))
    }

    fn reply(&mut self, client_id: u32, sequence: u64, data: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .reply(client_id, sequence, data.as_bytes())
            .map_err(map_err)
    }
}
