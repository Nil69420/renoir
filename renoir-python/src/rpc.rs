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
    fn set_stamp_requests(&mut self, stamp: bool) {
        self.inner.set_stamp_requests(stamp);
    }

    fn servers_alive(&mut self) -> PyResult<usize> {
        self.inner.servers_alive().map_err(map_err)
    }

    #[pyo3(signature = (timeout_ms=None))]
    fn wait_for_server(&mut self, py: Python<'_>, timeout_ms: Option<u64>) -> PyResult<bool> {
        let timeout = timeout_ms.map(std::time::Duration::from_millis);
        let inner = &mut self.inner;
        py.detach(|| inner.wait_for_server(timeout))
            .map_err(map_err)
    }

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
    #[pyo3(signature = (lifespan_ms=None))]
    fn set_request_lifespan(&mut self, lifespan_ms: Option<u64>) {
        self.inner
            .set_request_lifespan(lifespan_ms.map(std::time::Duration::from_millis));
    }

    fn stale_requests(&self) -> u64 {
        self.inner.stale_requests()
    }

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
