mod client;
mod reap;
mod server;

pub use client::RpcClient;
pub use server::{RpcRequest, RpcServer};

use crate::error::{RenoirError, Result};
use crate::topic_rings::lean_slot_bytes;

pub const RPC_HEADER_BYTES: usize = 16;
pub const RPC_MAX_PAYLOAD: usize = 504 - RPC_HEADER_BYTES;
pub(crate) const RESPONSE_SLOTS: usize = 16;
const MAX_REQUEST_SLOTS: usize = 1 << 16;

pub(crate) fn message_size(max_payload: usize) -> Result<usize> {
    if max_payload == 0 || max_payload > RPC_MAX_PAYLOAD {
        return Err(RenoirError::invalid_parameter(
            "max_payload",
            "RPC payload must be between 1 and 488 bytes",
        ));
    }
    Ok(RPC_HEADER_BYTES + max_payload)
}

pub(crate) fn check_slots(slots: usize) -> Result<()> {
    if !(2..=MAX_REQUEST_SLOTS).contains(&slots) {
        return Err(RenoirError::invalid_parameter(
            "slots",
            "RPC request ring needs between 2 and 65536 slots",
        ));
    }
    Ok(())
}

pub const RPC_MAX_SERVERS: usize = 4;

pub(crate) fn request_ring_name(service: &str, shard: usize) -> String {
    if shard == 0 {
        format!("{service}_req")
    } else {
        format!("{service}_req{shard}")
    }
}

pub(crate) fn response_ring_name(service: &str, client_id: u32) -> String {
    format!("{service}_rsp_{client_id:08x}")
}

pub(crate) fn encode(buf: &mut [u8], client_id: u32, seq: u64, payload: &[u8]) {
    buf[0..4].copy_from_slice(&client_id.to_le_bytes());
    buf[4..6].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    buf[6..8].copy_from_slice(&0u16.to_le_bytes());
    buf[8..16].copy_from_slice(&seq.to_le_bytes());
    buf[RPC_HEADER_BYTES..RPC_HEADER_BYTES + payload.len()].copy_from_slice(payload);
}

pub(crate) fn decode(buf: &[u8], max_payload: usize) -> Option<(u32, u64, usize)> {
    let len = usize::from(u16::from_le_bytes(buf[4..6].try_into().ok()?));
    if len > max_payload || RPC_HEADER_BYTES + len > buf.len() {
        return None;
    }
    let client_id = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    let seq = u64::from_le_bytes(buf[8..16].try_into().ok()?);
    Some((client_id, seq, len))
}

const SEQUENCE_BITS: u32 = 24;
const SEQUENCE_MASK: u64 = (1 << SEQUENCE_BITS) - 1;

pub(crate) fn pack_sequence(counter: u64, sent_ms: u64) -> u64 {
    (sent_ms << SEQUENCE_BITS) | (counter & SEQUENCE_MASK)
}

pub(crate) fn sent_ms(wire_sequence: u64) -> u64 {
    wire_sequence >> SEQUENCE_BITS
}

pub(crate) fn monotonic_ms() -> u64 {
    crate::topic_rings::monotonic_ns() / 1_000_000
}

pub(crate) fn check_ring_fits(msg_size: usize) -> Result<()> {
    lean_slot_bytes(msg_size)
        .map(|_| ())
        .ok_or_else(|| RenoirError::invalid_parameter("max_payload", "Message exceeds lean slot"))
}
