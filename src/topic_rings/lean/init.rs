use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    error::{RenoirError, Result},
    topic_rings::{
        init_guard::{claim, seal, Claim},
        spsc::SESSION_DIRTY,
    },
};

use super::{
    gate::GateEntry, LeanHeader, LEAN_FIXED_HEADER_BYTES, LEAN_MAX_CONSUMERS, LEAN_RING_MAGIC,
};

/// # Safety
///
/// `header` must point to a writable, 8-byte aligned `LeanHeader` in memory that stays
/// mapped for the duration of the call.
pub(super) unsafe fn attach_header(
    header: *mut LeanHeader,
    slot_count: u64,
    slot_bytes: u64,
    msg_size: u64,
    mode: u32,
) -> Result<()> {
    let magic = &*(header as *const AtomicU64);
    match claim(magic, LEAN_RING_MAGIC)? {
        Claim::Attach => {
            let h = &*header;
            if h.slot_count != slot_count || h.slot_bytes != slot_bytes || h.msg_size != msg_size {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing lean ring has a different geometry",
                ));
            }
            if h.mode != mode {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing lean ring uses a different producer mode",
                ));
            }
        }
        Claim::Create => {
            std::ptr::addr_of_mut!((*header).slot_count).write(slot_count);
            std::ptr::addr_of_mut!((*header).slot_bytes).write(slot_bytes);
            std::ptr::addr_of_mut!((*header).msg_size).write(msg_size);
            std::ptr::addr_of_mut!((*header).mode).write(mode);
            (*header).session.store(SESSION_DIRTY, Ordering::Relaxed);
            (*header).write_idx.store(0, Ordering::Relaxed);
            (*header).wake.store(0, Ordering::Relaxed);
            (*header).waiters.store(0, Ordering::Relaxed);
            (*header).claim.store(0, Ordering::Relaxed);
            for i in 0..LEAN_MAX_CONSUMERS {
                let entry = &*(header as *const u8)
                    .add(LEAN_FIXED_HEADER_BYTES)
                    .cast::<GateEntry>()
                    .add(i);
                entry.owner.store(0, Ordering::Relaxed);
                entry.pos.store(0, Ordering::Relaxed);
            }
            seal(magic, LEAN_RING_MAGIC);
        }
    }
    Ok(())
}
