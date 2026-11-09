use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanClock {
    None = 0,
    Nanoseconds = 1,
    Ticks = 2,
}

impl LeanClock {
    pub(super) fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::None),
            1 => Some(Self::Nanoseconds),
            2 => Some(Self::Ticks),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_stamped(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[inline]
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1_000_000_000 + ts.tv_nsec as u64
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
#[inline(always)]
pub fn ticks() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
#[inline(always)]
pub fn ticks() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntvct_el0", out(reg) value, options(nomem, nostack, preserves_flags))
    };
    value
}

#[cfg(any(miri, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
#[inline(always)]
pub fn ticks() -> u64 {
    monotonic_ns()
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
fn counter_hz() -> Option<u64> {
    let value: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) value, options(nomem, nostack, preserves_flags))
    };
    (value != 0).then_some(value)
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
fn counter_hz() -> Option<u64> {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    unsafe {
        if __cpuid(0x8000_0000).eax < 0x8000_0007 || __cpuid(0x8000_0007).edx & (1 << 8) == 0 {
            return None;
        }
        if __cpuid(0).eax >= 0x15 {
            let leaf = __cpuid_count(0x15, 0);
            if leaf.eax != 0 && leaf.ebx != 0 && leaf.ecx != 0 {
                return Some(u64::from(leaf.ecx) * u64::from(leaf.ebx) / u64::from(leaf.eax));
            }
        }
    }
    calibrate()
}

#[cfg(any(miri, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
fn counter_hz() -> Option<u64> {
    Some(1_000_000_000)
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
fn calibrate() -> Option<u64> {
    let mut best: Option<(u64, u64, u64)> = None;
    for _ in 0..64 {
        let t0 = monotonic_ns();
        let c0 = ticks();
        let t1 = monotonic_ns();
        let window = t1 - t0;
        if best.is_none_or(|(w, _, _)| window < w) {
            best = Some((window, c0, (t0 + t1) / 2));
        }
    }
    let (_, c_start, t_start) = best?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut end: Option<(u64, u64, u64)> = None;
    for _ in 0..64 {
        let t0 = monotonic_ns();
        let c0 = ticks();
        let t1 = monotonic_ns();
        let window = t1 - t0;
        if end.is_none_or(|(w, _, _)| window < w) {
            end = Some((window, c0, (t0 + t1) / 2));
        }
    }
    let (_, c_end, t_end) = end?;
    let elapsed_ns = t_end.checked_sub(t_start).filter(|d| *d > 0)?;
    let cycles = c_end.checked_sub(c_start)?;
    Some(((u128::from(cycles) * 1_000_000_000) / u128::from(elapsed_ns)) as u64)
}

pub fn tick_hz() -> Option<u64> {
    static HZ: OnceLock<Option<u64>> = OnceLock::new();
    *HZ.get_or_init(counter_hz)
}
