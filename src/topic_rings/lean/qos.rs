use std::time::Duration;

use super::{LeanChannel, LeanMode};
use crate::error::{RenoirError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanReliability {
    BestEffort,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeanQos {
    pub reliability: LeanReliability,
    pub depth: usize,
    pub deadline: Option<Duration>,
}

impl LeanQos {
    #[must_use]
    pub const fn best_effort(depth: usize) -> Self {
        Self {
            reliability: LeanReliability::BestEffort,
            depth,
            deadline: None,
        }
    }

    #[must_use]
    pub const fn reliable(depth: usize) -> Self {
        Self {
            reliability: LeanReliability::Reliable,
            depth,
            deadline: None,
        }
    }

    #[must_use]
    pub const fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineWait {
    Ready,
    Missed,
}

impl LeanChannel {
    pub fn open_qos(
        name: &str,
        msg_size: usize,
        qos: LeanQos,
        multi_producer: bool,
        subscriber: bool,
    ) -> Result<Self> {
        if qos.depth < 2 {
            return Err(RenoirError::invalid_parameter(
                "depth",
                "History depth must be at least 2",
            ));
        }
        let reliable = qos.reliability == LeanReliability::Reliable;
        let mut channel = Self::open_with_mode(
            name,
            msg_size,
            qos.depth,
            subscriber && reliable,
            LeanMode::new(multi_producer, reliable),
        )?;
        if subscriber && reliable {
            channel.register_consumer(true)?;
        }
        channel.set_deadline(qos.deadline);
        Ok(channel)
    }

    pub fn set_deadline(&mut self, deadline: Option<Duration>) {
        self.deadline = deadline;
    }

    pub fn deadline_misses(&self) -> u64 {
        self.deadline_misses
    }

    pub fn wait_deadline(&mut self, spin_iterations: u32) -> DeadlineWait {
        if self.wait_readable(spin_iterations, self.deadline) {
            DeadlineWait::Ready
        } else {
            self.deadline_misses += 1;
            DeadlineWait::Missed
        }
    }
}
