use std::{
    sync::atomic::{fence, Ordering},
    time::{Duration, Instant},
};

use super::{LeanCursor, LeanPublisher, LeanRing};
use crate::{error::Result, topic_rings::futex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanWaitStrategy {
    BusySpin = 0,
    Yielding = 1,
    Sleeping = 2,
    Blocking = 3,
    PhasedBackoff = 4,
}

impl LeanWaitStrategy {
    #[must_use]
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::BusySpin),
            1 => Some(Self::Yielding),
            2 => Some(Self::Sleeping),
            3 => Some(Self::Blocking),
            4 => Some(Self::PhasedBackoff),
            _ => None,
        }
    }
}

pub const PHASED_SPIN: Duration = Duration::from_micros(50);
pub const PHASED_YIELD: Duration = Duration::from_millis(1);
const SLEEP_STEP: Duration = Duration::from_micros(50);
const POLLS_PER_CLOCK_READ: u32 = 128;

impl LeanRing {
    pub fn wait_with(
        &self,
        cursor: &LeanCursor,
        strategy: LeanWaitStrategy,
        timeout: Option<Duration>,
    ) -> bool {
        match strategy {
            LeanWaitStrategy::Blocking => return self.wait_readable(cursor, 0, timeout),
            LeanWaitStrategy::PhasedBackoff => {
                return self.wait_phased(cursor, PHASED_SPIN, PHASED_YIELD, timeout)
            }
            _ => {}
        }
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        let mut polls = 0u32;
        loop {
            if cursor.next <= self.published() {
                return true;
            }
            polls += 1;
            if polls.is_multiple_of(POLLS_PER_CLOCK_READ)
                && deadline.is_some_and(|d| Instant::now() >= d)
            {
                return false;
            }
            match strategy {
                LeanWaitStrategy::BusySpin
                | LeanWaitStrategy::Blocking
                | LeanWaitStrategy::PhasedBackoff => std::hint::spin_loop(),
                LeanWaitStrategy::Yielding => std::thread::yield_now(),
                LeanWaitStrategy::Sleeping => {
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return false;
                    }
                    std::thread::sleep(SLEEP_STEP);
                }
            }
        }
    }

    pub fn wait_phased(
        &self,
        cursor: &LeanCursor,
        spin_for: Duration,
        yield_for: Duration,
        timeout: Option<Duration>,
    ) -> bool {
        let start = Instant::now();
        let deadline = timeout.and_then(|t| start.checked_add(t));
        let mut polls = 0u32;
        loop {
            if cursor.next <= self.published() {
                return true;
            }
            polls = polls.wrapping_add(1);
            if !polls.is_multiple_of(POLLS_PER_CLOCK_READ) {
                std::hint::spin_loop();
                continue;
            }
            let now = Instant::now();
            if deadline.is_some_and(|d| now >= d) {
                return false;
            }
            let elapsed = now.duration_since(start);
            if elapsed >= spin_for.saturating_add(yield_for) {
                break;
            }
            if elapsed >= spin_for {
                std::thread::yield_now();
            }
        }
        let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        self.wait_readable(cursor, 0, remaining)
    }

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
