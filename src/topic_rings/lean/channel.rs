use std::{sync::Arc, time::Duration};

use crate::{
    error::{RenoirError, Result},
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
};

use super::{
    lean_slot_bytes, stamped::STAMP_BYTES, LeanClock, LeanCursor, LeanGatedCursor, LeanMode,
    LeanPublisher, LeanRead, LeanRing,
};

const SPIN_START: u32 = 1_000;
const SPIN_MIN: u32 = 32;
const SPIN_MAX: u32 = 100_000;

pub struct LeanChannel {
    ring: Arc<LeanRing>,
    publisher: LeanPublisher,
    cursor: LeanCursor,
    gated: Option<LeanGatedCursor>,
    pub(super) deadline: Option<Duration>,
    pub(super) deadline_misses: u64,
    spin_budget: u32,
    announced: bool,
    _region: SharedMemoryRegion,
}

impl LeanChannel {
    pub fn open(name: &str, msg_size: usize, slot_count: usize, from_oldest: bool) -> Result<Self> {
        Self::open_with_mode(
            name,
            msg_size,
            slot_count,
            from_oldest,
            LeanMode::SingleProducer,
        )
    }

    pub fn open_with_mode(
        name: &str,
        msg_size: usize,
        slot_count: usize,
        from_oldest: bool,
        mode: LeanMode,
    ) -> Result<Self> {
        Self::open_clocked(
            name,
            msg_size,
            slot_count,
            from_oldest,
            mode,
            LeanClock::None,
        )
    }

    pub fn open_stamped(
        name: &str,
        data_size: usize,
        slot_count: usize,
        from_oldest: bool,
        mode: LeanMode,
        clock: LeanClock,
    ) -> Result<Self> {
        if !clock.is_stamped() {
            return Err(RenoirError::invalid_parameter(
                "clock",
                "A stamped channel needs a nanosecond or tick clock",
            ));
        }
        Self::open_clocked(name, data_size, slot_count, from_oldest, mode, clock)
    }

    fn open_clocked(
        name: &str,
        msg_size: usize,
        slot_count: usize,
        from_oldest: bool,
        mode: LeanMode,
        clock: LeanClock,
    ) -> Result<Self> {
        let msg_size = if clock.is_stamped() {
            msg_size.checked_add(STAMP_BYTES).ok_or_else(|| {
                RenoirError::invalid_parameter("msg_size", "Message size overflows")
            })?
        } else {
            msg_size
        };
        let slot_bytes = lean_slot_bytes(msg_size).ok_or_else(|| {
            RenoirError::invalid_parameter("msg_size", "Lean message size must be 1..=504 bytes")
        })?;
        if slot_count == 0 {
            return Err(RenoirError::invalid_parameter(
                "slot_count",
                "Lean slot count must be non-zero",
            ));
        }
        let ring_bytes = slot_bytes
            .checked_mul(slot_count)
            .ok_or_else(|| RenoirError::invalid_parameter("slot_count", "Ring size overflows"))?;
        let size = LeanRing::required_memory_size(ring_bytes, msg_size);
        let region = SharedMemoryRegion::new(RegionConfig {
            name: name.to_owned(),
            size,
            backing_type: BackingType::FileBacked,
            ..RegionConfig::default()
        })?;
        let ptr = std::ptr::NonNull::new(unsafe { region.as_mut_ptr_unsafe::<u8>() })
            .ok_or_else(|| RenoirError::invalid_parameter("region", "Region mapping is null"))?;
        let ring = Arc::new(unsafe {
            if clock.is_stamped() {
                LeanRing::from_memory_stamped(
                    ptr,
                    region.size(),
                    ring_bytes,
                    msg_size - STAMP_BYTES,
                    mode,
                    clock,
                )?
            } else {
                LeanRing::from_memory_mode(ptr, region.size(), ring_bytes, msg_size, mode)?
            }
        });
        let cursor = if from_oldest {
            ring.cursor_oldest()
        } else {
            ring.cursor_latest()
        };
        let publisher = ring.publisher();
        Ok(Self {
            ring,
            publisher,
            cursor,
            gated: None,
            deadline: None,
            deadline_misses: 0,
            spin_budget: SPIN_START,
            announced: false,
            _region: region,
        })
    }

    pub fn register_consumer(&mut self, from_oldest: bool) -> Result<()> {
        if let Some(mut old) = self.gated.take() {
            self.ring.unregister_consumer(&mut old);
        }
        self.gated = Some(self.ring.register_consumer(from_oldest)?);
        Ok(())
    }

    fn active_cursor(&self) -> &LeanCursor {
        match &self.gated {
            Some(gated) => &gated.cursor,
            None => &self.cursor,
        }
    }

    pub fn ring(&self) -> &Arc<LeanRing> {
        &self.ring
    }

    #[inline]
    pub fn publish(&mut self, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish(data)
    }

    #[inline]
    pub fn publish_at(&mut self, stamp: u64, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish_at(stamp, data)
    }

    #[inline]
    pub fn publish_now(&mut self, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish_now(data)
    }

    #[inline]
    pub fn publish_at_notify(&mut self, stamp: u64, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish_at_notify(stamp, data)
    }

    #[inline]
    pub fn publish_now_notify(&mut self, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish_now_notify(data)
    }

    #[inline]
    pub fn try_read_stamped(&mut self, out: &mut [u8]) -> Result<Option<u64>> {
        match self.gated.as_mut() {
            Some(gated) => self.ring.try_read_stamped_gated(gated, out),
            None => self.ring.try_read_stamped(&mut self.cursor, out),
        }
    }

    #[inline]
    pub fn publish_notify(&mut self, data: &[u8]) -> Result<()> {
        self.announce();
        self.publisher.publish_notify(data)
    }

    #[inline]
    pub fn publish_batch(&mut self, data: &[u8]) -> Result<usize> {
        self.announce();
        self.publisher.publish_batch(data)
    }

    pub fn read_batch(&mut self, out: &mut [u8]) -> Result<usize> {
        match self.gated.as_mut() {
            Some(gated) => self.ring.read_batch_gated(gated, out),
            None => self.ring.read_batch(&mut self.cursor, out),
        }
    }

    pub fn wait_adaptive(&mut self, timeout: Option<Duration>) -> bool {
        if self.wait_readable(self.spin_budget, Some(Duration::ZERO)) {
            self.spin_budget = (self.spin_budget.saturating_mul(2)).min(SPIN_MAX);
            return true;
        }
        self.spin_budget = (self.spin_budget / 2).max(SPIN_MIN);
        self.wait_readable(0, timeout)
    }

    #[must_use]
    pub fn spin_budget(&self) -> u32 {
        self.spin_budget
    }

    pub fn wait_readable(&self, spin_iterations: u32, timeout: Option<Duration>) -> bool {
        self.ring
            .wait_readable(self.active_cursor(), spin_iterations, timeout)
    }

    pub fn wait_with(&self, strategy: super::LeanWaitStrategy, timeout: Option<Duration>) -> bool {
        self.ring.wait_with(self.active_cursor(), strategy, timeout)
    }

    pub fn wait_phased(
        &self,
        spin_for: Duration,
        yield_for: Duration,
        timeout: Option<Duration>,
    ) -> bool {
        self.ring
            .wait_phased(self.active_cursor(), spin_for, yield_for, timeout)
    }

    #[inline]
    pub fn try_read(&mut self, out: &mut [u8]) -> Result<LeanRead> {
        match self.gated.as_mut() {
            Some(gated) => self.ring.try_read_gated(gated, out),
            None => self.ring.try_read(&mut self.cursor, out),
        }
    }

    #[inline]
    pub fn try_read_lagged(&mut self, out: &mut [u8]) -> Result<(LeanRead, u64)> {
        let before = self.lost();
        let read = self.try_read(out)?;
        Ok((read, self.lost() - before))
    }

    #[inline]
    pub fn try_read_stamped_lagged(&mut self, out: &mut [u8]) -> Result<(Option<u64>, u64)> {
        let before = self.lost();
        let stamp = self.try_read_stamped(out)?;
        Ok((stamp, self.lost() - before))
    }

    pub fn skip_to_latest(&mut self, keep: u64) -> u64 {
        match self.gated.as_mut() {
            Some(gated) => self.ring.skip_to_latest_gated(gated, keep),
            None => self.ring.skip_to_latest(&mut self.cursor, keep),
        }
    }

    pub fn lost(&self) -> u64 {
        self.active_cursor().lost()
    }

    pub fn pending(&self) -> usize {
        self.ring.pending(self.active_cursor())
    }

    #[inline]
    fn announce(&mut self) {
        if !self.announced {
            self.announced = true;
            self.publisher.announce();
        }
    }

    #[must_use]
    pub fn consumer_id(&self) -> Option<usize> {
        self.gated.as_ref().and_then(LeanGatedCursor::consumer_id)
    }

    pub fn set_upstream(&mut self, upstream: usize) -> Result<()> {
        let Some(gated) = self.gated.as_mut() else {
            return Err(RenoirError::invalid_parameter(
                "channel",
                "Only registered consumers can have an upstream",
            ));
        };
        self.ring.set_upstream(gated, upstream)
    }

    #[must_use]
    pub fn live_consumers(&self) -> usize {
        self.ring.live_consumers()
    }

    #[must_use]
    pub fn live_consumers_before_me(&self) -> usize {
        self.consumer_id()
            .map_or(0, |id| self.ring.live_consumers_before(id))
    }

    pub fn unregister_consumer(&mut self) {
        if let Some(mut gated) = self.gated.take() {
            self.ring.unregister_consumer(&mut gated);
        }
    }

    pub fn live_publishers(&self) -> usize {
        self.ring.live_publishers()
    }

    pub fn msg_size(&self) -> usize {
        self.ring.msg_size()
    }
}

impl Drop for LeanChannel {
    fn drop(&mut self) {
        if let Some(gated) = self.gated.as_mut() {
            self.ring.unregister_consumer(gated);
        }
    }
}
