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
}

impl BroadcastCursor {
    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub fn next_index(&self) -> u64 {
        self.next
    }
}
