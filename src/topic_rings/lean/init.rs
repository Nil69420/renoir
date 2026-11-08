use std::{
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::{
    error::{RenoirError, Result},
    topic_rings::spsc::SESSION_DIRTY,
};

use super::{LeanHeader, LEAN_RING_MAGIC};

const INITIALIZING: u64 = 0x4C45_414E_5F49_4E49;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

/// # Safety
///
/// `header` must point to a writable, 8-byte aligned `LeanHeader` in memory that stays
/// mapped for the duration of the call.
pub(super) unsafe fn attach_header(
    header: *mut LeanHeader,
    slot_count: u64,
    slot_bytes: u64,
    msg_size: u64,
) -> Result<()> {
    let magic = &*(header as *const AtomicU64);
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        let seen = magic.load(Ordering::Acquire);
        if seen == LEAN_RING_MAGIC {
            let h = &*header;
            if h.slot_count != slot_count || h.slot_bytes != slot_bytes || h.msg_size != msg_size {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing lean ring has a different geometry",
                ));
            }
            return Ok(());
        }
        if seen == INITIALIZING {
            if Instant::now() >= deadline {
                return Err(RenoirError::concurrency(
                    "Timed out waiting for another process to initialise the lean ring",
                ));
            }
            thread::sleep(Duration::from_micros(100));
            continue;
        }
        if magic
            .compare_exchange(seen, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            std::ptr::addr_of_mut!((*header).slot_count).write(slot_count);
            std::ptr::addr_of_mut!((*header).slot_bytes).write(slot_bytes);
            std::ptr::addr_of_mut!((*header).msg_size).write(msg_size);
            (*header).session.store(SESSION_DIRTY, Ordering::Relaxed);
            (*header).write_idx.store(0, Ordering::Relaxed);
            magic.store(LEAN_RING_MAGIC, Ordering::Release);
            return Ok(());
        }
    }
}
