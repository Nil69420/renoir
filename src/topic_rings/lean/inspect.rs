use super::{
    gate::{self, GateEntry, LEAN_MAX_CONSUMERS},
    liveness::{self, LEAN_MAX_PRODUCERS},
    LeanClock, LeanHeader, LeanMode, LEAN_FIXED_HEADER_BYTES, LEAN_HEADER_BYTES, LEAN_RING_MAGIC,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeanInfo {
    pub slot_count: u64,
    pub slot_bytes: u64,
    pub msg_size: u64,
    pub mode: LeanMode,
    pub clock: LeanClock,
    pub tick_hz: u64,
    pub published: u64,
    pub live_publishers: usize,
    pub registered_consumers: usize,
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub fn inspect(bytes: &[u8]) -> Option<LeanInfo> {
    if bytes.len() < LEAN_HEADER_BYTES
        || u64_at(bytes, std::mem::offset_of!(LeanHeader, magic))? != LEAN_RING_MAGIC
    {
        return None;
    }
    let mode = match u32_at(bytes, std::mem::offset_of!(LeanHeader, mode))? {
        0 => LeanMode::SingleProducer,
        1 => LeanMode::MultiProducer,
        2 => LeanMode::SingleProducerGated,
        3 => LeanMode::MultiProducerGated,
        _ => return None,
    };
    let clock = LeanClock::from_raw(u32_at(bytes, std::mem::offset_of!(LeanHeader, clock))?)?;
    let registered_consumers = (0..LEAN_MAX_CONSUMERS)
        .filter(|i| {
            let at = LEAN_FIXED_HEADER_BYTES
                + i * std::mem::size_of::<GateEntry>()
                + std::mem::offset_of!(GateEntry, owner);
            u64_at(bytes, at).is_some_and(|o| o & gate::STATE_MASK == gate::ACTIVE)
        })
        .count();
    let table = LEAN_FIXED_HEADER_BYTES + gate::GATE_TABLE_BYTES;
    let live_publishers = (0..LEAN_MAX_PRODUCERS)
        .filter(|i| {
            let base = table + i * std::mem::size_of::<liveness::ProducerEntry>();
            let field = |name: usize| base + name;
            let owner = u64_at(
                bytes,
                field(std::mem::offset_of!(liveness::ProducerEntry, owner)),
            );
            let pid = u32_at(
                bytes,
                field(std::mem::offset_of!(liveness::ProducerEntry, pid)),
            );
            let start = u64_at(
                bytes,
                field(std::mem::offset_of!(liveness::ProducerEntry, start)),
            );
            match (owner, pid, start) {
                (Some(o), Some(pid), Some(start)) => {
                    o & liveness::STATE_MASK == liveness::ACTIVE
                        && liveness::process_alive(pid, start)
                }
                _ => false,
            }
        })
        .count();
    Some(LeanInfo {
        slot_count: u64_at(bytes, std::mem::offset_of!(LeanHeader, slot_count))?,
        slot_bytes: u64_at(bytes, std::mem::offset_of!(LeanHeader, slot_bytes))?,
        msg_size: u64_at(bytes, std::mem::offset_of!(LeanHeader, msg_size))?,
        mode,
        clock,
        tick_hz: u64_at(bytes, std::mem::offset_of!(LeanHeader, tick_hz))?,
        published: u64_at(bytes, std::mem::offset_of!(LeanHeader, write_idx))?,
        live_publishers,
        registered_consumers,
    })
}
