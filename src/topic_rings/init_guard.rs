use std::{
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::error::Result;

const INITIALIZING: u64 = 0x4942_4E49_5F42_5254;
const TAKEOVER_AFTER: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Claim {
    Create,
    Attach,
}

pub(super) fn claim(magic: &AtomicU64, final_magic: u64) -> Result<Claim> {
    let deadline = Instant::now() + TAKEOVER_AFTER;
    loop {
        let seen = magic.load(Ordering::Acquire);
        if seen == final_magic {
            return Ok(Claim::Attach);
        }
        if seen == INITIALIZING {
            if Instant::now() >= deadline {
                return Ok(Claim::Create);
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
