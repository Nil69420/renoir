use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use crate::{
    error::{RenoirError, Result},
    topic_rings::{LeanChannel, LeanMode, LeanRead},
};

use super::{
    check_ring_fits, check_slots, decode, encode, message_size, request_ring_name,
    response_ring_name, RESPONSE_SLOTS, RPC_HEADER_BYTES,
};

const SPIN_ITERATIONS: u32 = 2000;

pub struct RpcClient {
    client_id: u32,
    sequence: u64,
    max_payload: usize,
    requests: LeanChannel,
    responses: LeanChannel,
    scratch: Vec<u8>,
}

fn new_client_id() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mixed = std::process::id().wrapping_mul(0x9E37_79B1)
        ^ nanos.rotate_left(13)
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x85EB_CA6B);
    mixed.max(1)
}

impl RpcClient {
    pub fn open(service: &str, max_payload: usize, request_slots: usize) -> Result<Self> {
        let msg = message_size(max_payload)?;
        check_ring_fits(msg)?;
        check_slots(request_slots)?;
        let client_id = new_client_id();
        let requests = LeanChannel::open_with_mode(
            &request_ring_name(service),
            msg,
            request_slots,
            false,
            LeanMode::MultiProducerGated,
        )?;
        let responses = LeanChannel::open(
            &response_ring_name(service, client_id),
            msg,
            RESPONSE_SLOTS,
            false,
        )?;
        Ok(Self {
            client_id,
            sequence: 0,
            max_payload,
            requests,
            responses,
            scratch: vec![0u8; msg],
        })
    }

    pub fn client_id(&self) -> u32 {
        self.client_id
    }

    pub fn call(
        &mut self,
        request: &[u8],
        response: &mut [u8],
        timeout: Duration,
    ) -> Result<usize> {
        if request.len() > self.max_payload {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Request larger than the service maximum payload",
            ));
        }
        let deadline = Instant::now().checked_add(timeout);
        self.sequence += 1;
        let sequence = self.sequence;
        encode(&mut self.scratch, self.client_id, sequence, request);
        while self.requests.publish_notify(&self.scratch).is_err() {
            if expired(deadline) {
                return Err(RenoirError::buffer_full("rpc request"));
            }
            std::thread::yield_now();
        }
        loop {
            match self.responses.try_read(&mut self.scratch)? {
                LeanRead::Message => {
                    let Some((_, seq, len)) = decode(&self.scratch, self.max_payload) else {
                        continue;
                    };
                    if seq != sequence {
                        continue;
                    }
                    if response.len() < len {
                        return Err(RenoirError::invalid_parameter(
                            "buffer",
                            "Response buffer smaller than the response",
                        ));
                    }
                    response[..len].copy_from_slice(&self.scratch[RPC_HEADER_BYTES..][..len]);
                    return Ok(len);
                }
                LeanRead::Empty => {
                    let remaining = match deadline {
                        Some(d) => match d.checked_duration_since(Instant::now()) {
                            Some(r) => Some(r),
                            None => return Err(RenoirError::buffer_empty("rpc response")),
                        },
                        None => None,
                    };
                    self.responses.wait_readable(SPIN_ITERATIONS, remaining);
                }
            }
        }
    }
}

fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}
