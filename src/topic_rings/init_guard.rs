use std::{
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::error::{RenoirError, Result};

const INITIALIZING: u64 = 0x4942_4E49_5F42_5254;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Claim {
    Create,
    Attach,
}

pub(super) fn claim(magic: &AtomicU64, final_magic: u64) -> Result<Claim> {
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        let seen = magic.load(Ordering::Acquire);
        if seen == final_magic {
            return Ok(Claim::Attach);
        }
        if seen == INITIALIZING {
            if Instant::now() >= deadline {
                return Err(RenoirError::concurrency(
                    "Timed out waiting for another opener to initialise the ring",
                ));
            }
            thread::sleep(Duration::from_micros(100));
            continue;
        }
        if magic
            .compare_exchange(seen, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(Claim::Create);
        }
    }
}

pub(super) fn seal(magic: &AtomicU64, final_magic: u64) {
    magic.store(final_magic, Ordering::Release);
}
