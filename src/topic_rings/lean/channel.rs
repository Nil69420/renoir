use std::{sync::Arc, time::Duration};

use crate::{
    error::{RenoirError, Result},
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
};

use super::{
    lean_slot_bytes, LeanCursor, LeanGatedCursor, LeanMode, LeanPublisher, LeanRead, LeanRing,
};

pub struct LeanChannel {
    ring: Arc<LeanRing>,
    publisher: LeanPublisher,
    cursor: LeanCursor,
    gated: Option<LeanGatedCursor>,
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
            LeanRing::from_memory_mode(ptr, region.size(), ring_bytes, msg_size, mode)?
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
        self.publisher.publish(data)
    }

    #[inline]
    pub fn publish_notify(&mut self, data: &[u8]) -> Result<()> {
        self.publisher.publish_notify(data)
    }

    #[inline]
    pub fn publish_batch(&mut self, data: &[u8]) -> Result<usize> {
        self.publisher.publish_batch(data)
    }

    pub fn read_batch(&mut self, out: &mut [u8]) -> Result<usize> {
        match self.gated.as_mut() {
            Some(gated) => self.ring.read_batch_gated(gated, out),
            None => self.ring.read_batch(&mut self.cursor, out),
        }
    }

    pub fn wait_readable(&self, spin_iterations: u32, timeout: Option<Duration>) -> bool {
        self.ring
            .wait_readable(self.active_cursor(), spin_iterations, timeout)
    }

    #[inline]
    pub fn try_read(&mut self, out: &mut [u8]) -> Result<LeanRead> {
        match self.gated.as_mut() {
            Some(gated) => self.ring.try_read_gated(gated, out),
            None => self.ring.try_read(&mut self.cursor, out),
        }
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
