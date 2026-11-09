pub const RENOIR_IMAGE_FRAME_MAGIC: u32 = 0x474D_4952;
pub const RENOIR_IMAGE_FRAME_VERSION: u16 = 1;
pub const RENOIR_ENCODING_MONO8: u16 = 0;
pub const RENOIR_ENCODING_MONO16: u16 = 1;
pub const RENOIR_ENCODING_RGB8: u16 = 2;
pub const RENOIR_ENCODING_BGR8: u16 = 3;
pub const RENOIR_DISTORTION_NONE: u32 = 0;
pub const RENOIR_DISTORTION_PLUMB_BOB: u32 = 1;
pub const RENOIR_DISTORTION_EQUIDISTANT: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirImageFrameHeader {
    pub magic: u32,
    pub version: u16,
    pub encoding: u16,
    pub stamp_ns: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub camera_id: u32,
    pub frame_seq: u64,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub distortion_model: u32,
    pub flags: u32,
    pub distortion: [f64; 5],
    pub pixel_offset: u32,
    pub reserved: u32,
}

pub const IMAGE_FRAME_HEADER_BYTES: usize = std::mem::size_of::<RenoirImageFrameHeader>();

const _: () = assert!(IMAGE_FRAME_HEADER_BYTES == 128);
const _: () = assert!(std::mem::align_of::<RenoirImageFrameHeader>() == 8);
const _: () = assert!(std::mem::offset_of!(RenoirImageFrameHeader, stamp_ns) == 8);
const _: () = assert!(std::mem::offset_of!(RenoirImageFrameHeader, fx) == 40);
const _: () = assert!(std::mem::offset_of!(RenoirImageFrameHeader, pixel_offset) == 120);

pub const fn bytes_per_pixel(encoding: u16) -> Option<u32> {
    match encoding {
        RENOIR_ENCODING_MONO8 => Some(1),
        RENOIR_ENCODING_MONO16 => Some(2),
        RENOIR_ENCODING_RGB8 | RENOIR_ENCODING_BGR8 => Some(3),
        _ => None,
    }
}

pub fn frame_payload_size(width: u32, height: u32, stride: u32, encoding: u16) -> Option<usize> {
    let bpp = bytes_per_pixel(encoding)?;
    let min_stride = width.checked_mul(bpp)?;
    let stride = if stride == 0 { min_stride } else { stride };
    if width == 0 || height == 0 || stride < min_stride {
        return None;
    }
    let pixels = (stride as usize).checked_mul(height as usize)?;
    IMAGE_FRAME_HEADER_BYTES.checked_add(pixels)
}

pub fn validate_frame(payload: &[u8]) -> bool {
    if payload.len() < IMAGE_FRAME_HEADER_BYTES {
        return false;
    }
    // SAFETY: length checked above; read_unaligned has no alignment requirement and
    // every bit pattern is a valid RenoirImageFrameHeader (integers and floats only).
    let h: RenoirImageFrameHeader =
        unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const RenoirImageFrameHeader) };
    if h.magic != RENOIR_IMAGE_FRAME_MAGIC || h.version != RENOIR_IMAGE_FRAME_VERSION {
        return false;
    }
    let Some(total) = frame_payload_size(h.width, h.height, h.stride, h.encoding) else {
        return false;
    };
    h.pixel_offset as usize == IMAGE_FRAME_HEADER_BYTES && payload.len() >= total
}

pub fn init_header(
    width: u32,
    height: u32,
    stride: u32,
    encoding: u16,
    stamp_ns: u64,
    camera_id: u32,
    frame_seq: u64,
) -> Option<RenoirImageFrameHeader> {
    let bpp = bytes_per_pixel(encoding)?;
    frame_payload_size(width, height, stride, encoding)?;
    Some(RenoirImageFrameHeader {
        magic: RENOIR_IMAGE_FRAME_MAGIC,
        version: RENOIR_IMAGE_FRAME_VERSION,
        encoding,
        stamp_ns,
        width,
        height,
        stride: if stride == 0 { width * bpp } else { stride },
        camera_id,
        frame_seq,
        fx: 0.0,
        fy: 0.0,
        cx: 0.0,
        cy: 0.0,
        distortion_model: RENOIR_DISTORTION_NONE,
        flags: 0,
        distortion: [0.0; 5],
        pixel_offset: IMAGE_FRAME_HEADER_BYTES as u32,
        reserved: 0,
    })
}
