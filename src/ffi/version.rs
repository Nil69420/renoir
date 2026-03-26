//! FFI version information functions

use std::ffi::c_char;

use super::utils::string_to_c_str;

/// Get major version number
#[no_mangle]
pub extern "C" fn renoir_version_major() -> u32 {
    crate::VERSION_MAJOR
}

/// Get minor version number
#[no_mangle]
pub extern "C" fn renoir_version_minor() -> u32 {
    crate::VERSION_MINOR
}

/// Get patch version number
#[no_mangle]
pub extern "C" fn renoir_version_patch() -> u32 {
    crate::VERSION_PATCH
}

/// Get version string (caller must free with renoir_free_string)
#[no_mangle]
pub extern "C" fn renoir_version_string() -> *mut c_char {
    string_to_c_str(crate::VERSION.to_string())
}
