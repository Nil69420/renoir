use std::{
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    error::{RenoirError, Result},
    topic_rings::{
        init_guard::{claim, seal, Claim},
        spsc::SESSION_DIRTY,
    },
};

use super::{BroadcastHeader, BROADCAST_RING_MAGIC};

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
    match claim(magic, BROADCAST_RING_MAGIC)? {
        Claim::Attach => {
            let h = &*header;
            if h.slot_count != slot_count || h.stride != stride || h.payload_cap != payload_cap {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing broadcast ring has a different geometry",
                ));
            }
        }
        Claim::Create => {
            ptr::addr_of_mut!((*header).slot_count).write(slot_count);
            (*header).session.store(SESSION_DIRTY, Ordering::Relaxed);
            ptr::addr_of_mut!((*header).stride).write(stride);
            ptr::addr_of_mut!((*header).payload_cap).write(payload_cap);
            (*header).write_idx.store(0, Ordering::Relaxed);
            (*header).wake.store(0, Ordering::Relaxed);
            (*header).waiters.store(0, Ordering::Relaxed);
            seal(magic, BROADCAST_RING_MAGIC);
        }
    }
    Ok(())
}
