pub const RENOIR_IMU_SAMPLE_MAGIC: u32 = 0x554D_4952;
pub const RENOIR_IMU_SAMPLE_VERSION: u16 = 1;
pub const RENOIR_IMU_FLAG_ORIENTATION_VALID: u16 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirImuSample {
    pub magic: u32,
    pub version: u16,
    pub flags: u16,
    pub stamp_ns: u64,
    pub gyro: [f64; 3],
    pub accel: [f64; 3],
    pub orientation: [f64; 4],
    pub seq: u64,
    pub imu_id: u32,
    pub reserved0: u32,
    pub reserved: [u64; 2],
}

pub const IMU_SAMPLE_BYTES: usize = std::mem::size_of::<RenoirImuSample>();

const _: () = assert!(IMU_SAMPLE_BYTES == 128);
const _: () = assert!(std::mem::align_of::<RenoirImuSample>() == 8);
const _: () = assert!(std::mem::offset_of!(RenoirImuSample, gyro) == 16);
const _: () = assert!(std::mem::offset_of!(RenoirImuSample, orientation) == 64);
const _: () = assert!(std::mem::offset_of!(RenoirImuSample, seq) == 96);

pub fn validate_sample(payload: &[u8]) -> bool {
    if payload.len() < IMU_SAMPLE_BYTES {
        return false;
    }
    // SAFETY: length checked above; read_unaligned has no alignment requirement and
    // every bit pattern is a valid RenoirImuSample (integers and floats only).
    let s: RenoirImuSample =
        unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const RenoirImuSample) };
    s.magic == RENOIR_IMU_SAMPLE_MAGIC && s.version == RENOIR_IMU_SAMPLE_VERSION
}

pub fn init_sample(
    stamp_ns: u64,
    gyro: [f64; 3],
    accel: [f64; 3],
    imu_id: u32,
    seq: u64,
) -> RenoirImuSample {
    RenoirImuSample {
        magic: RENOIR_IMU_SAMPLE_MAGIC,
        version: RENOIR_IMU_SAMPLE_VERSION,
        flags: 0,
        stamp_ns,
        gyro,
        accel,
        orientation: [1.0, 0.0, 0.0, 0.0],
        seq,
        imu_id,
        reserved0: 0,
        reserved: [0; 2],
    }
}
