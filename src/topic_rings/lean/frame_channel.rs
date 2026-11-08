use std::{sync::Arc, time::Duration};

use crate::{
    error::{RenoirError, Result},
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
};

use super::{
    frame_slot_bytes, FrameLoan, FrameView, LeanGatedCursor, LeanMode, LeanPublisher, LeanRing,
};

pub struct FrameChannel {
    ring: Arc<LeanRing>,
    publisher: LeanPublisher,
    consumer: Option<LeanGatedCursor>,
    pending_loan: Option<(u64, usize)>,
    pending_view: Option<u64>,
    stale_dropped: u64,
    _region: SharedMemoryRegion,
}

impl FrameChannel {
    pub fn open(
        name: &str,
        max_frame: usize,
        slot_count: usize,
        multi_producer: bool,
        consumer: bool,
        from_oldest: bool,
    ) -> Result<Self> {
        let slot_bytes = frame_slot_bytes(max_frame).ok_or_else(|| {
            RenoirError::invalid_parameter("max_frame", "Frame size must be 1..=64 MiB")
        })?;
        if slot_count < 2 {
            return Err(RenoirError::invalid_parameter(
                "slot_count",
                "Frame ring needs at least 2 slots",
            ));
        }
        let ring_bytes = slot_bytes
            .checked_mul(slot_count)
            .ok_or_else(|| RenoirError::invalid_parameter("slot_count", "Ring size overflows"))?;
        let size = LeanRing::required_frame_memory(ring_bytes, max_frame);
        let region = SharedMemoryRegion::new(RegionConfig {
            name: name.to_owned(),
            size,
            backing_type: BackingType::FileBacked,
            ..RegionConfig::default()
        })?;
        let ptr = std::ptr::NonNull::new(unsafe { region.as_mut_ptr_unsafe::<u8>() })
            .ok_or_else(|| RenoirError::invalid_parameter("region", "Region mapping is null"))?;
        let mode = LeanMode::new(multi_producer, true);
        let ring = Arc::new(unsafe {
            LeanRing::from_frame_memory(ptr, region.size(), ring_bytes, max_frame, mode)?
        });
        let consumer = if consumer {
            Some(ring.register_consumer(from_oldest)?)
        } else {
            None
        };
        let publisher = ring.publisher();
        Ok(Self {
            ring,
            publisher,
            consumer,
            pending_loan: None,
            pending_view: None,
            stale_dropped: 0,
            _region: region,
        })
    }

    #[must_use]
    pub fn max_frame(&self) -> usize {
        self.ring.max_frame()
    }

    pub fn loan(&mut self, len: usize) -> Result<FrameLoan<'_>> {
        self.publisher.announce();
        self.publisher.loan_frame(len)
    }

    pub fn view(&mut self) -> Result<Option<FrameView<'_>>> {
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        self.ring.view_frame(cursor)
    }

    pub fn view_fresh(&mut self, max_age: Duration) -> Result<Option<FrameView<'_>>> {
        let limit = u32::try_from(max_age.as_micros()).unwrap_or(u32::MAX);
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        loop {
            match self.ring.peek_frame(cursor)? {
                None => return Ok(None),
                Some((idx, _)) if self.ring.frame_age_micros(idx) > limit => {
                    self.ring.release_frame(cursor, idx);
                    self.stale_dropped += 1;
                }
                Some((idx, len)) => {
                    return Ok(Some(FrameView::new(&self.ring, cursor, idx, len)));
                }
            }
        }
    }

    #[must_use]
    pub fn stale_dropped(&self) -> u64 {
        self.stale_dropped
    }

    pub fn view_fresh_raw(&mut self, max_age: Duration) -> Result<Option<(*const u8, usize)>> {
        if self.pending_view.is_some() {
            return Err(RenoirError::invalid_parameter(
                "view",
                "A frame view is already held",
            ));
        }
        let limit = u32::try_from(max_age.as_micros()).unwrap_or(u32::MAX);
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        loop {
            match self.ring.peek_frame(cursor)? {
                None => return Ok(None),
                Some((idx, _)) if self.ring.frame_age_micros(idx) > limit => {
                    self.ring.release_frame(cursor, idx);
                    self.stale_dropped += 1;
                }
                Some((idx, len)) => {
                    self.pending_view = Some(idx);
                    return Ok(Some((self.ring.frame_ptr(idx), len)));
                }
            }
        }
    }

    pub fn loan_raw(&mut self, len: usize) -> Result<*mut u8> {
        if self.pending_loan.is_some() {
            return Err(RenoirError::invalid_parameter(
                "loan",
                "A frame is already on loan",
            ));
        }
        self.publisher.announce();
        let idx = self.publisher.begin_frame(len)?;
        self.pending_loan = Some((idx, len));
        Ok(self.publisher.frame_buffer(idx))
    }

    pub fn commit_raw(&mut self, notify: bool) -> Result<()> {
        let (idx, len) = self.pending_loan.take().ok_or_else(no_loan)?;
        self.publisher.end_frame(idx, Some(len))?;
        if notify {
            self.ring.notify();
        }
        Ok(())
    }

    pub fn abort_raw(&mut self) -> Result<()> {
        let (idx, _) = self.pending_loan.take().ok_or_else(no_loan)?;
        self.publisher.end_frame(idx, None)
    }

    pub fn view_raw(&mut self) -> Result<Option<(*const u8, usize)>> {
        if self.pending_view.is_some() {
            return Err(RenoirError::invalid_parameter(
                "view",
                "A frame view is already held",
            ));
        }
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        Ok(self.ring.peek_frame(cursor)?.map(|(idx, len)| {
            self.pending_view = Some(idx);
            (self.ring.frame_ptr(idx), len)
        }))
    }

    pub fn release_raw(&mut self) -> Result<()> {
        let idx = self
            .pending_view
            .take()
            .ok_or_else(|| RenoirError::invalid_parameter("view", "No frame view is held"))?;
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        self.ring.release_frame(cursor, idx);
        Ok(())
    }

    pub fn view_intact_raw(&self) -> bool {
        match (&self.consumer, self.pending_view) {
            (Some(cursor), Some(idx)) => self.ring.frame_intact(cursor, idx),
            _ => false,
        }
    }

    pub fn wait(&mut self, spin_iterations: u32, timeout: Option<Duration>) -> bool {
        match self.consumer.as_ref() {
            Some(cursor) => self
                .ring
                .wait_readable(&cursor.cursor, spin_iterations, timeout),
            None => false,
        }
    }

    #[must_use]
    pub fn consumer_id(&self) -> Option<usize> {
        self.consumer
            .as_ref()
            .and_then(LeanGatedCursor::consumer_id)
    }

    pub fn set_upstream(&mut self, upstream: usize) -> Result<()> {
        let cursor = self.consumer.as_mut().ok_or_else(not_a_consumer)?;
        self.ring.set_upstream(cursor, upstream)
    }

    #[must_use]
    pub fn live_publishers(&self) -> usize {
        self.ring.live_publishers()
    }
}

fn not_a_consumer() -> RenoirError {
    RenoirError::invalid_parameter("channel", "Channel was not opened as a consumer")
}

fn no_loan() -> RenoirError {
    RenoirError::invalid_parameter("loan", "No frame is on loan")
}

impl Drop for FrameChannel {
    fn drop(&mut self) {
        if let Some((idx, _)) = self.pending_loan.take() {
            let _ = self.publisher.end_frame(idx, None);
        }
        if let (Some(idx), Some(cursor)) = (self.pending_view.take(), self.consumer.as_mut()) {
            self.ring.release_frame(cursor, idx);
        }
        if let Some(cursor) = self.consumer.as_mut() {
            self.ring.unregister_consumer(cursor);
        }
    }
}
