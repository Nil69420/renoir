use std::{
    sync::atomic::{fence, Ordering},
    time::{Duration, Instant},
};

use super::{LeanCursor, LeanPublisher, LeanRing};
use crate::{error::Result, topic_rings::futex};

impl LeanRing {
    pub fn wait_readable(
        &self,
        cursor: &LeanCursor,
        spin_iterations: u32,
        timeout: Option<Duration>,
    ) -> bool {
        for _ in 0..spin_iterations {
            if cursor.next <= self.published() {
                return true;
            }
            std::hint::spin_loop();
        }
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        let h = self.hdr();
        loop {
            if cursor.next <= self.published() {
                return true;
            }
            let token = h.wake.load(Ordering::Acquire);
            h.waiters.fetch_add(1, Ordering::SeqCst);
            let ready = cursor.next <= self.write_idx().load(Ordering::SeqCst);
            let mut timed_out = false;
            if !ready {
                match deadline.map(|d| d.checked_duration_since(Instant::now())) {
                    Some(None) => timed_out = true,
                    Some(Some(remaining)) => futex::wait(&h.wake, token, Some(remaining)),
                    None => futex::wait(&h.wake, token, None),
                }
            }
            h.waiters.fetch_sub(1, Ordering::SeqCst);
            if ready {
                return true;
            }
            if timed_out {
                return false;
            }
        }
    }

    pub(super) fn notify(&self) {
        fence(Ordering::SeqCst);
        let h = self.hdr();
        if h.waiters.load(Ordering::Relaxed) != 0 {
            h.wake.fetch_add(1, Ordering::Release);
            futex::wake_all(&h.wake);
        }
    }
}

impl LeanPublisher {
    #[inline]
    pub fn publish_notify(&mut self, data: &[u8]) -> Result<()> {
        self.publish(data)?;
        self.ring.notify();
        Ok(())
    }
}
