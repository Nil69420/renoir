pub(super) fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_ne_bytes(raw))
}

pub(super) fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_ne_bytes(raw))
}

pub(super) fn read_usize(bytes: &[u8], offset: usize) -> Option<usize> {
    const N: usize = std::mem::size_of::<usize>();
    let raw: [u8; N] = bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()?;
    Some(usize::from_ne_bytes(raw))
}

pub(super) fn valid_capacity(capacity: usize, memory_len: usize) -> bool {
    capacity != 0 && capacity.is_power_of_two() && capacity <= memory_len
}
