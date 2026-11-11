use std::time::Duration;

use super::{
    liveliness::{self, LivelinessStatus},
    DeadlineWait, FrameChannel, FrameView, LeanClock, LeanDurability, LeanOwnership, LeanQos,
    LeanReliability,
};
use crate::error::{RenoirError, Result};

impl FrameChannel {
    pub fn open_qos(
        name: &str,
        max_frame: usize,
        qos: LeanQos,
        multi_producer: bool,
        consumer: bool,
    ) -> Result<Self> {
        qos.validate_with(false)?;
        let bad =
            |what: &'static str, why: &'static str| Err(RenoirError::invalid_parameter(what, why));
        if qos.reliability != LeanReliability::Reliable {
            return bad(
                "reliability",
                "Frame rings are lossless: ask for reliable delivery",
            );
        }
        if qos.clock != LeanClock::None {
            return bad(
                "clock",
                "Frames carry their own commit timestamp; do not set a clock",
            );
        }
        if qos.min_separation.is_some() {
            return bad(
                "min_separation",
                "A time-based filter is not offered on frame rings",
            );
        }
        let exclusive = matches!(qos.ownership, LeanOwnership::Exclusive { .. });
        let from_oldest = qos.durability == LeanDurability::TransientLocal;
        let mut channel = Self::open(
            name,
            max_frame,
            qos.depth,
            multi_producer || exclusive,
            consumer,
            from_oldest,
        )?;
        if consumer {
            channel.ring.check_offer(&qos)?;
        } else {
            channel.ring.offer(&qos)?;
        }
        channel.deadline = qos.deadline;
        let ring = channel.ring.clone();
        liveliness::configure(&mut channel.qos, &ring, &qos, consumer);
        channel.fast = false;
        Ok(channel)
    }

    pub fn register_publisher(&mut self) -> Result<()> {
        let ring = self.ring.clone();
        liveliness::register(&ring, &mut self.publisher, &self.qos)
    }

    pub fn assert_liveliness(&mut self) -> Result<()> {
        let ring = self.ring.clone();
        liveliness::assert_now(&ring, &mut self.publisher, &self.qos)
    }

    pub fn liveliness(&mut self) -> LivelinessStatus {
        let ring = self.ring.clone();
        liveliness::status(&ring, &mut self.qos)
    }

    pub fn wait_liveliness_change(
        &mut self,
        timeout: Option<Duration>,
    ) -> Option<LivelinessStatus> {
        let ring = self.ring.clone();
        liveliness::wait_change(&ring, &mut self.qos, timeout)
    }

    pub fn is_owner(&mut self) -> bool {
        liveliness::is_owner(&self.ring, &self.publisher, &self.qos)
    }

    pub fn lifespan(&self) -> Option<Duration> {
        (self.qos.lifespan_ns != 0).then(|| Duration::from_nanos(self.qos.lifespan_ns))
    }

    pub fn view_qos(&mut self) -> Result<Option<FrameView<'_>>> {
        match self.lifespan() {
            Some(lifespan) => self.view_fresh(lifespan),
            None => self.view(),
        }
    }

    pub fn view_qos_raw(&mut self) -> Result<Option<(*const u8, usize)>> {
        match self.lifespan() {
            Some(lifespan) => self.view_fresh_raw(lifespan),
            None => self.view_raw(),
        }
    }

    pub fn deadline_misses(&self) -> u64 {
        self.deadline_misses
    }

    pub fn wait_deadline(&mut self, spin_iterations: u32) -> DeadlineWait {
        if self.wait(spin_iterations, self.deadline) {
            DeadlineWait::Ready
        } else {
            self.deadline_misses += 1;
            DeadlineWait::Missed
        }
    }
}
