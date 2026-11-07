use std::sync::atomic::AtomicU32;
use std::time::Duration;

#[cfg(target_os = "linux")]
pub(super) fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    let ts = timeout.map(|d| libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: d.subsec_nanos() as libc::c_long,
    });
    let ts_ptr = ts
        .as_ref()
        .map_or(std::ptr::null(), |t| t as *const libc::timespec);
    // SAFETY: word points to a live 4-byte aligned futex word in mapped memory and
    // ts_ptr is either null or points to a timespec that outlives the call.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            word.as_ptr(),
            libc::FUTEX_WAIT,
            expected,
            ts_ptr,
            std::ptr::null_mut::<u32>(),
            0u32,
        );
    }
}

#[cfg(target_os = "linux")]
pub(super) fn wake_all(word: &AtomicU32) {
    // SAFETY: word points to a live 4-byte aligned futex word in mapped memory.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            word.as_ptr(),
            libc::FUTEX_WAKE,
            i32::MAX,
            std::ptr::null::<libc::timespec>(),
            std::ptr::null_mut::<u32>(),
            0u32,
        );
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn wait(_word: &AtomicU32, _expected: u32, timeout: Option<Duration>) {
    let pause = Duration::from_micros(50);
    std::thread::sleep(timeout.map_or(pause, |t| t.min(pause)));
}

#[cfg(not(target_os = "linux"))]
pub(super) fn wake_all(_word: &AtomicU32) {}
