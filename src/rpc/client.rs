use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use crate::{
    error::{RenoirError, Result},
    memory::config::region_path,
    topic_rings::{LeanChannel, LeanMode, LeanRead},
};

use super::{
    check_ring_fits, check_slots, decode, encode, message_size,
    reap::{reap_abandoned, ResponseLease},
    request_ring_name, response_ring_name, RESPONSE_SLOTS, RPC_HEADER_BYTES, RPC_MAX_SERVERS,
};

const SPIN_ITERATIONS: u32 = 2000;
const FAILOVER_SLICE: Duration = Duration::from_millis(5);

pub struct RpcClient {
    service: String,
    msg_size: usize,
    request_slots: usize,
    client_id: u32,
    sequence: u64,
    max_payload: usize,
    shards: Vec<(usize, LeanChannel)>,
    current: usize,
    responses: LeanChannel,
    _lease: ResponseLease,
    request_buf: Vec<u8>,
    scratch: Vec<u8>,
    stamp: bool,
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
        let first = open_shard(service, 0, msg, request_slots)?;
        reap_abandoned(service);
        let response_name = response_ring_name(service, client_id);
        let responses = LeanChannel::open(&response_name, msg, RESPONSE_SLOTS, false)?;
        let lease = ResponseLease::acquire(&response_name)?;
        let mut client = Self {
            service: service.to_owned(),
            msg_size: msg,
            request_slots,
            client_id,
            sequence: 0,
            max_payload,
            shards: vec![(0, first)],
            current: 0,
            responses,
            _lease: lease,
            request_buf: vec![0u8; msg],
            scratch: vec![0u8; msg],
            stamp: false,
        };
        client.reroute()?;
        Ok(client)
    }

    fn reroute(&mut self) -> Result<()> {
        for shard in 1..RPC_MAX_SERVERS {
            let known = self.shards.iter().any(|(s, _)| *s == shard);
            if !known && region_path(&request_ring_name(&self.service, shard)).exists() {
                let channel = open_shard(&self.service, shard, self.msg_size, self.request_slots)?;
                self.shards.push((shard, channel));
            }
        }
        let live: Vec<usize> = (0..self.shards.len())
            .filter(|&i| self.shards[i].1.live_consumers() > 0)
            .collect();
        self.current = if live.is_empty() {
            0
        } else {
            live[self.client_id as usize % live.len()]
        };
        Ok(())
    }

    #[must_use]
    pub fn shard(&self) -> usize {
        self.shards[self.current].0
    }

    pub fn set_stamp_requests(&mut self, stamp: bool) {
        self.stamp = stamp;
    }

    pub fn servers_alive(&mut self) -> Result<usize> {
        self.reroute()?;
        Ok(self
            .shards
            .iter()
            .filter(|(_, channel)| channel.live_consumers() > 0)
            .count())
    }

    pub fn wait_for_server(&mut self, timeout: Option<Duration>) -> Result<bool> {
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        loop {
            if self.servers_alive()? > 0 {
                return Ok(true);
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
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
        let sequence =
            super::pack_sequence(sequence, if self.stamp { super::monotonic_ms() } else { 0 });
        encode(&mut self.request_buf, self.client_id, sequence, request);
        self.send(deadline)?;
        let mut next_check = Instant::now() + FAILOVER_SLICE;
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
                    let now = Instant::now();
                    if now >= next_check {
                        next_check = now + FAILOVER_SLICE;
                        if self.shards[self.current].1.live_consumers() == 0 {
                            let before = self.current;
                            self.reroute()?;
                            if self.current != before {
                                self.send(deadline)?;
                            }
                        }
                    }
                    let remaining = match deadline {
                        Some(d) => match d.checked_duration_since(Instant::now()) {
                            Some(r) => Some(r.min(FAILOVER_SLICE)),
                            None => return Err(RenoirError::buffer_empty("rpc response")),
                        },
                        None => Some(FAILOVER_SLICE),
                    };
                    self.responses.wait_readable(SPIN_ITERATIONS, remaining);
                }
            }
        }
    }
}

impl RpcClient {
    fn send(&mut self, deadline: Option<Instant>) -> Result<()> {
        while self.shards[self.current]
            .1
            .publish_notify(&self.request_buf)
            .is_err()
        {
            if expired(deadline) {
                return Err(RenoirError::buffer_full("rpc request"));
            }
            std::thread::yield_now();
        }
        Ok(())
    }
}

fn open_shard(service: &str, shard: usize, msg: usize, slots: usize) -> Result<LeanChannel> {
    LeanChannel::open_with_mode(
        &request_ring_name(service, shard),
        msg,
        slots,
        false,
        LeanMode::MultiProducerGated,
    )
}

fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}
