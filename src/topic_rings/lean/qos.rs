use std::time::Duration;

use super::{LeanChannel, LeanClock, LeanMode};
use crate::error::{RenoirError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanReliability {
    BestEffort,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanHistory {
    KeepLast,
    KeepAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanDurability {
    Volatile,
    TransientLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanLivelinessKind {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeanLiveliness {
    pub kind: LeanLivelinessKind,
    pub lease: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanOwnership {
    Shared,
    Exclusive { strength: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeanQos {
    pub reliability: LeanReliability,
    pub depth: usize,
    pub deadline: Option<Duration>,
    pub history: LeanHistory,
    pub durability: LeanDurability,
    pub lifespan: Option<Duration>,
    pub liveliness: LeanLiveliness,
    pub ownership: LeanOwnership,
    pub min_separation: Option<Duration>,
    pub clock: LeanClock,
    pub require_offer: bool,
}

impl LeanQos {
    const fn base(reliability: LeanReliability, depth: usize) -> Self {
        Self {
            reliability,
            depth,
            deadline: None,
            history: LeanHistory::KeepLast,
            durability: LeanDurability::Volatile,
            lifespan: None,
            liveliness: LeanLiveliness {
                kind: LeanLivelinessKind::Automatic,
                lease: None,
            },
            ownership: LeanOwnership::Shared,
            min_separation: None,
            clock: LeanClock::None,
            require_offer: false,
        }
    }

    #[must_use]
    pub const fn best_effort(depth: usize) -> Self {
        Self::base(LeanReliability::BestEffort, depth)
    }

    #[must_use]
    pub const fn reliable(depth: usize) -> Self {
        Self::base(LeanReliability::Reliable, depth)
    }

    #[must_use]
    pub const fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    #[must_use]
    pub const fn keep_all(mut self) -> Self {
        self.history = LeanHistory::KeepAll;
        self
    }

    #[must_use]
    pub const fn transient_local(mut self) -> Self {
        self.durability = LeanDurability::TransientLocal;
        self
    }

    #[must_use]
    pub const fn with_clock(mut self, clock: LeanClock) -> Self {
        self.clock = clock;
        self
    }

    #[must_use]
    pub const fn require_offer(mut self) -> Self {
        self.require_offer = true;
        self
    }

    #[must_use]
    pub const fn with_lifespan(mut self, lifespan: Duration) -> Self {
        self.lifespan = Some(lifespan);
        self
    }

    #[must_use]
    pub const fn with_min_separation(mut self, separation: Duration) -> Self {
        self.min_separation = Some(separation);
        self
    }

    #[must_use]
    pub const fn with_manual_liveliness(mut self, lease: Duration) -> Self {
        self.liveliness = LeanLiveliness {
            kind: LeanLivelinessKind::Manual,
            lease: Some(lease),
        };
        self
    }

    #[must_use]
    pub const fn exclusive(mut self, strength: u64) -> Self {
        self.ownership = LeanOwnership::Exclusive { strength };
        self
    }

    pub(super) fn validate(&self) -> Result<()> {
        let bad =
            |what: &'static str, why: &'static str| Err(RenoirError::invalid_parameter(what, why));
        if self.depth < 2 {
            return bad("depth", "History depth must be at least 2");
        }
        if self.history == LeanHistory::KeepAll && self.reliability != LeanReliability::Reliable {
            return bad("history", "Keep-all history needs reliable delivery");
        }
        if (self.lifespan.is_some() || self.min_separation.is_some()) && !self.clock.is_stamped() {
            return bad(
                "clock",
                "Lifespan and time-based filtering need a stamped ring",
            );
        }
        if self.liveliness.kind == LeanLivelinessKind::Manual
            && self.liveliness.lease.is_none_or(|l| l.is_zero())
        {
            return bad("liveliness", "Manual liveliness needs a non-zero lease");
        }
        if self.deadline.is_some_and(|d| d.is_zero()) || self.lifespan.is_some_and(|d| d.is_zero())
        {
            return bad("qos", "Deadline and lifespan must be non-zero");
        }
        Ok(())
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
        qos.validate()?;
        let reliable = qos.reliability == LeanReliability::Reliable;
        let from_oldest =
            subscriber && (reliable || qos.durability == LeanDurability::TransientLocal);
        let exclusive = matches!(qos.ownership, LeanOwnership::Exclusive { .. });
        let mut channel = Self::open_clocked(
            name,
            msg_size,
            qos.depth,
            from_oldest,
            LeanMode::new(multi_producer || exclusive, reliable),
            qos.clock,
        )?;
        if subscriber {
            channel.ring().check_offer(&qos)?;
        } else {
            channel.ring().offer(&qos)?;
        }
        if subscriber && reliable {
            channel.register_consumer(true)?;
        }
        channel.apply_qos(&qos, subscriber);
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
