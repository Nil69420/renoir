use std::{
    ptr,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::{
    error::{RenoirError, Result},
    topic_rings::spsc::SESSION_DIRTY,
};

use super::{BroadcastHeader, BROADCAST_RING_MAGIC};

const INITIALIZING: u64 = 0x4942_4E49_5F42_5254;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

/// # Safety
///
/// `header` must point to a writable, 8-byte aligned `BroadcastHeader` in memory that
/// stays mapped for the duration of the call.
pub(super) unsafe fn attach_header(
    header: *mut BroadcastHeader,
    slot_count: usize,
    stride: usize,
    payload_cap: usize,
) -> Result<()> {
    let magic = &*(header as *const AtomicU64);
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        let seen = magic.load(Ordering::Acquire);
        if seen == BROADCAST_RING_MAGIC {
            let h = &*header;
            if h.slot_count != slot_count || h.stride != stride || h.payload_cap != payload_cap {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing broadcast ring has a different geometry",
                ));
            }
            return Ok(());
        }
        if seen == INITIALIZING {
            if Instant::now() >= deadline {
                return Err(RenoirError::concurrency(
                    "Timed out waiting for another process to initialise the broadcast ring",
                ));
            }
            thread::sleep(Duration::from_micros(100));
            continue;
        }
        if magic
            .compare_exchange(seen, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            ptr::addr_of_mut!((*header).slot_count).write(slot_count);
            (*header).session.store(SESSION_DIRTY, Ordering::Relaxed);
            ptr::addr_of_mut!((*header).stride).write(stride);
            ptr::addr_of_mut!((*header).payload_cap).write(payload_cap);
            (*header).write_idx.store(0, Ordering::Relaxed);
            (*header).wake.store(0, Ordering::Relaxed);
            (*header).waiters.store(0, Ordering::Relaxed);
            magic.store(BROADCAST_RING_MAGIC, Ordering::Release);
            return Ok(());
        }
    }
}
