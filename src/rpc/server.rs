use std::{collections::HashMap, time::Duration};

use crate::{
    error::{RenoirError, Result},
    topic_rings::{LeanChannel, LeanMode, LeanRead},
};

use super::{
    check_ring_fits, check_slots, decode, encode, message_size, request_ring_name,
    response_ring_name, RESPONSE_SLOTS, RPC_MAX_SERVERS,
};

const MAX_CACHED_CLIENTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcRequest {
    pub client_id: u32,
    pub sequence: u64,
    pub len: usize,
}

pub struct RpcServer {
    service: String,
    shard: usize,
    max_payload: usize,
    requests: LeanChannel,
    responses: HashMap<u32, LeanChannel>,
    scratch: Vec<u8>,
}

impl RpcServer {
    pub fn open(service: &str, max_payload: usize, request_slots: usize) -> Result<Self> {
        let msg = message_size(max_payload)?;
        check_ring_fits(msg)?;
        check_slots(request_slots)?;
        for shard in 0..RPC_MAX_SERVERS {
            let mut requests = LeanChannel::open_with_mode(
                &request_ring_name(service, shard),
                msg,
                request_slots,
                false,
                LeanMode::MultiProducerGated,
            )?;
            requests.register_consumer(false)?;
            if requests.live_consumers_before_me() == 0 {
                return Ok(Self {
                    service: service.to_owned(),
                    shard,
                    max_payload,
                    requests,
                    responses: HashMap::with_capacity(MAX_CACHED_CLIENTS),
                    scratch: vec![0u8; msg],
                });
            }
            requests.unregister_consumer();
        }
        Err(RenoirError::concurrency(
            "All RPC server shards for this service are taken",
        ))
    }

    #[must_use]
    pub fn shard(&self) -> usize {
        self.shard
    }

    pub fn poll(&mut self, payload: &mut [u8]) -> Result<Option<RpcRequest>> {
        loop {
            match self.requests.try_read(&mut self.scratch)? {
                LeanRead::Empty => return Ok(None),
                LeanRead::Message => {
                    let Some((client_id, sequence, len)) = decode(&self.scratch, self.max_payload)
                    else {
                        continue;
                    };
                    if payload.len() < len {
                        return Err(RenoirError::invalid_parameter(
                            "buffer",
                            "Request buffer smaller than the request",
                        ));
                    }
                    payload[..len].copy_from_slice(&self.scratch[super::RPC_HEADER_BYTES..][..len]);
                    return Ok(Some(RpcRequest {
                        client_id,
                        sequence,
                        len,
                    }));
                }
            }
        }
    }

    pub fn wait_request(&self, spin_iterations: u32, timeout: Option<Duration>) -> bool {
        self.requests.wait_readable(spin_iterations, timeout)
    }

    pub fn reply(&mut self, client_id: u32, sequence: u64, data: &[u8]) -> Result<()> {
        if data.len() > self.max_payload {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Response larger than the service maximum payload",
            ));
        }
        encode(&mut self.scratch, client_id, sequence, data);
        let msg_size = self.scratch.len();
        let channel = response_channel(&mut self.responses, &self.service, client_id, msg_size)?;
        channel.publish_notify(&self.scratch)
    }
}

fn response_channel<'a>(
    responses: &'a mut HashMap<u32, LeanChannel>,
    service: &str,
    client_id: u32,
    msg_size: usize,
) -> Result<&'a mut LeanChannel> {
    if !responses.contains_key(&client_id) {
        if responses.len() >= MAX_CACHED_CLIENTS {
            if let Some(oldest) = responses.keys().next().copied() {
                responses.remove(&oldest);
            }
        }
        let channel = LeanChannel::open(
            &response_ring_name(service, client_id),
            msg_size,
            RESPONSE_SLOTS,
            false,
        )?;
        responses.insert(client_id, channel);
    }
    responses
        .get_mut(&client_id)
        .ok_or_else(|| RenoirError::concurrency("Response channel cache lost an entry"))
}
