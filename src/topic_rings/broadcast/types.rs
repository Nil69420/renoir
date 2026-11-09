#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadInto {
    Empty,
    TooSmall(usize),
    Message {
        len: usize,
        sequence: u64,
        timestamp_ns: u64,
    },
}

#[derive(Debug)]
pub struct BroadcastReservation {
    pub(super) idx: u64,
    pub(super) payload: *mut u8,
    pub(super) capacity: usize,
}

unsafe impl Send for BroadcastReservation {}

impl BroadcastReservation {
    pub fn payload_ptr(&self) -> *mut u8 {
        self.payload
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BroadcastCursor {
    pub(super) next: u64,
    pub(super) overruns: u64,
    pub(super) skipped: u64,
}

impl BroadcastCursor {
    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    pub fn next_index(&self) -> u64 {
        self.next
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BroadcastView {
    pub(super) ptr: *const u8,
    pub(super) len: usize,
    pub(super) sequence: u64,
    pub(super) timestamp_ns: u64,
    pub(super) index: u64,
}

unsafe impl Send for BroadcastView {}

impl BroadcastView {
    #[must_use]
    pub fn from_parts(
        ptr: *const u8,
        len: usize,
        sequence: u64,
        timestamp_ns: u64,
        index: u64,
    ) -> Self {
        Self {
            ptr,
            len,
            sequence,
            timestamp_ns,
            index,
        }
    }

    #[must_use]
    pub fn data_ptr(&self) -> *const u8 {
        self.ptr
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }

    #[must_use]
    pub fn index(&self) -> u64 {
        self.index
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ViewRead {
    Empty,
    View(BroadcastView),
}
