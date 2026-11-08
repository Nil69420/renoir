use std::{
    ffi::{c_char, c_void, CStr},
    time::Duration,
};

use crate::rpc::{RpcClient, RpcServer};

use super::types::RenoirErrorCode;

pub type RenoirRpcHandle = *mut c_void;

fn service_name<'a>(name: *const c_char) -> Option<&'a str> {
    if name.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(name) }.to_str().ok()
}

fn timeout_of(timeout_ns: u64) -> Option<Duration> {
    (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns))
}

#[no_mangle]
pub extern "C" fn renoir_rpc_client_open(
    service: *const c_char,
    max_payload: u32,
    request_slots: u32,
    out: *mut RenoirRpcHandle,
) -> RenoirErrorCode {
    let (Some(service), false) = (service_name(service), out.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match RpcClient::open(service, max_payload as usize, request_slots as usize) {
        Ok(client) => {
            unsafe { *out = Box::into_raw(Box::new(client)) as RenoirRpcHandle };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_call(
    client: RenoirRpcHandle,
    request: *const u8,
    request_len: usize,
    response: *mut u8,
    response_cap: usize,
    timeout_ns: u64,
    response_len: *mut usize,
) -> RenoirErrorCode {
    let Some(client) = (unsafe { (client as *mut RpcClient).as_mut() }) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if request.is_null() || response.is_null() || response_len.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let request = unsafe { std::slice::from_raw_parts(request, request_len) };
    let response = unsafe { std::slice::from_raw_parts_mut(response, response_cap) };
    let timeout = timeout_of(timeout_ns).unwrap_or(Duration::MAX);
    match client.call(request, response, timeout) {
        Ok(n) => {
            unsafe { *response_len = n };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_client_close(client: RenoirRpcHandle) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client as *mut RpcClient) });
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_server_open(
    service: *const c_char,
    max_payload: u32,
    request_slots: u32,
    out: *mut RenoirRpcHandle,
) -> RenoirErrorCode {
    let (Some(service), false) = (service_name(service), out.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match RpcServer::open(service, max_payload as usize, request_slots as usize) {
        Ok(server) => {
            unsafe { *out = Box::into_raw(Box::new(server)) as RenoirRpcHandle };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_poll(
    server: RenoirRpcHandle,
    buffer: *mut u8,
    buffer_cap: usize,
    client_id: *mut u32,
    sequence: *mut u64,
    len: *mut usize,
) -> RenoirErrorCode {
    let Some(server) = (unsafe { (server as *mut RpcServer).as_mut() }) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() || client_id.is_null() || sequence.is_null() || len.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_cap) };
    match server.poll(buffer) {
        Ok(Some(req)) => {
            unsafe {
                *client_id = req.client_id;
                *sequence = req.sequence;
                *len = req.len;
            }
            RenoirErrorCode::Success
        }
        Ok(None) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_wait_request(
    server: RenoirRpcHandle,
    spin_iterations: u32,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(server) = (unsafe { (server as *mut RpcServer).as_ref() }) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if server.wait_request(spin_iterations, timeout_of(timeout_ns)) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_reply(
    server: RenoirRpcHandle,
    client_id: u32,
    sequence: u64,
    data: *const u8,
    len: usize,
) -> RenoirErrorCode {
    let Some(server) = (unsafe { (server as *mut RpcServer).as_mut() }) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() && len != 0 {
        return RenoirErrorCode::InvalidParameter;
    }
    let data = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    match server.reply(client_id, sequence, data) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_rpc_server_close(server: RenoirRpcHandle) {
    if !server.is_null() {
        drop(unsafe { Box::from_raw(server as *mut RpcServer) });
    }
}
