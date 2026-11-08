use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GateHandle {
    pub(super) entry: usize,
    pub(super) token: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct LeanCursor {
    pub(super) next: u64,
    pub(super) lost: u64,
    pub(super) empty_polls: u32,
    pub(super) stalled: Option<(u64, Instant)>,
}

impl LeanCursor {
    pub(super) fn at(next: u64) -> Self {
        Self {
            next,
            lost: 0,
            empty_polls: 0,
            stalled: None,
        }
    }

    #[must_use]
    pub fn lost(&self) -> u64 {
        self.lost
    }

    #[must_use]
    pub fn next_index(&self) -> u64 {
        self.next
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LeanGatedCursor {
    pub(super) cursor: LeanCursor,
    pub(super) handle: Option<GateHandle>,
    pub(super) upstream: Option<GateHandle>,
}

impl LeanGatedCursor {
    #[must_use]
    pub fn lost(&self) -> u64 {
        self.cursor.lost
    }

    #[must_use]
    pub fn next_index(&self) -> u64 {
        self.cursor.next
    }

    #[must_use]
    pub fn is_registered(&self) -> bool {
        self.handle.is_some()
    }

    #[must_use]
    pub fn consumer_id(&self) -> Option<usize> {
        self.handle.map(|h| h.entry)
    }

    #[must_use]
    pub fn has_upstream(&self) -> bool {
        self.upstream.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanMode {
    SingleProducer = 0,
    MultiProducer = 1,
    SingleProducerGated = 2,
    MultiProducerGated = 3,
}

impl LeanMode {
    #[must_use]
    pub const fn new(multi_producer: bool, gated: bool) -> Self {
        match (multi_producer, gated) {
            (false, false) => Self::SingleProducer,
            (true, false) => Self::MultiProducer,
            (false, true) => Self::SingleProducerGated,
            (true, true) => Self::MultiProducerGated,
        }
    }

    #[must_use]
    pub const fn is_multi_producer(self) -> bool {
        self as u32 & 1 != 0
    }

    #[must_use]
    pub const fn is_gated(self) -> bool {
        self as u32 & 2 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanRead {
    Empty,
    Message,
}
