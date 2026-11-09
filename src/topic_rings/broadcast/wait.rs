use std::{
    sync::atomic::{fence, Ordering},
    time::{Duration, Instant},
};

use super::{types::BroadcastCursor, BroadcastTopicRing};
use crate::topic_rings::futex;

impl BroadcastTopicRing {
    pub(super) fn notify(&self) {
        fence(Ordering::SeqCst);
        let h = self.hdr();
        if h.waiters.load(Ordering::Relaxed) != 0 {
            h.wake.fetch_add(1, Ordering::Release);
            futex::wake_all(&h.wake);
        }
    }

    pub fn wait_readable(&self, cursor: &BroadcastCursor, timeout: Option<Duration>) -> bool {
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        let h = self.hdr();
        loop {
            if cursor.next <= self.write_idx().load(Ordering::Acquire) {
                return true;
            }
            let token = h.wake.load(Ordering::Acquire);
            h.waiters.fetch_add(1, Ordering::SeqCst);
            let ready = cursor.next <= self.write_idx().load(Ordering::SeqCst);
            let mut timed_out = false;
            if !ready {
                let remaining = match deadline {
                    Some(d) => {
                        let now = Instant::now();
                        if now >= d {
                            timed_out = true;
                            None
                        } else {
                            Some(d - now)
                        }
                    }
                    None => None,
                };
                if !timed_out {
                    futex::wait(&h.wake, token, remaining);
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
}
