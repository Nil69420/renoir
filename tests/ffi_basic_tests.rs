//! FFI (C API) Integration Tests
//! 
//! Tests for the C foreign function interface to validate
//! C/C++ integration capabilities

#[cfg(feature = "c-api")]
use renoir::ffi::{
    renoir_version_major, renoir_version_minor, renoir_version_patch, 
    renoir_version_string, renoir_free_string,
    renoir_manager_create, renoir_manager_destroy,
    RenoirErrorCode,
};
use std::ffi::CStr;
use std::ptr;

#[cfg(test)]
#[cfg(feature = "c-api")]
mod ffi_tests {
    use super::*;

    #[test]
    fn test_version_information() {
        unsafe {
            let major = renoir_version_major();
            let minor = renoir_version_minor();
            let patch = renoir_version_patch();
            
            // Version should be reasonable
            assert!(major < 100);
            assert!(minor < 100);
            assert!(patch < 1000);
            
            // Test version string
            let version_ptr = renoir_version_string();
            assert!(!version_ptr.is_null());
            
            let version_str = CStr::from_ptr(version_ptr).to_string_lossy();
            assert!(!version_str.is_empty());
            assert!(version_str.contains(&major.to_string()));
            
            // Clean up string
            renoir_free_string(version_ptr);
        }
    }

    #[test]
    fn test_manager_lifecycle() {
        // Create manager
        let manager = renoir_manager_create();
        assert!(!manager.is_null());
        
        // Destroy manager
        renoir_manager_destroy(manager);
    }

    #[test]
    fn test_null_pointer_safety() {
        // Test that passing null pointers doesn't crash
        
        // Null manager operations
        renoir_manager_destroy(ptr::null_mut()); // Should not crash
        
        // Null string operations
        renoir_free_string(ptr::null_mut()); // Should not crash
    }

    #[test]
    fn test_string_handling() {
        unsafe {
            let version_ptr = renoir_version_string();
            
            if !version_ptr.is_null() {
                // Verify string is valid UTF-8
                let version_cstr = CStr::from_ptr(version_ptr);
                let version_str = version_cstr.to_str();
                assert!(version_str.is_ok());
                
                // Clean up
                renoir_free_string(version_ptr);
            }
        }
    }

    #[test]
    fn test_concurrent_safety_basic() {
        // This is a basic test - real concurrent testing would need multiple threads
        let manager1 = renoir_manager_create();
        let manager2 = renoir_manager_create();
        
        // Both should be valid (or both null if creation fails)
        if !manager1.is_null() {
            assert!(!manager2.is_null());
            
            renoir_manager_destroy(manager1);
            renoir_manager_destroy(manager2);
        }
    }

    #[test]
    fn test_memory_management() {
        // Create and destroy multiple managers to test memory management
        for _ in 0..10 {
            let manager = renoir_manager_create();
            if !manager.is_null() {
                renoir_manager_destroy(manager);
            }
        }
    }

    #[test]
    fn test_error_code_values() {
        // Verify error code values are as expected
        assert_eq!(RenoirErrorCode::Success as i32, 0);
        assert_ne!(RenoirErrorCode::InvalidParameter as i32, 0);
        assert_ne!(RenoirErrorCode::OutOfMemory as i32, 0);
        
        // Ensure different error codes have different values
        assert_ne!(RenoirErrorCode::InvalidParameter, RenoirErrorCode::OutOfMemory);
    }

    #[test]
    fn test_version_consistency() {
        // Test that version functions return consistent values
        let major1 = renoir_version_major();
        let major2 = renoir_version_major();
        assert_eq!(major1, major2);
        
        let minor1 = renoir_version_minor();
        let minor2 = renoir_version_minor();
        assert_eq!(minor1, minor2);
        
        let patch1 = renoir_version_patch();
        let patch2 = renoir_version_patch();
        assert_eq!(patch1, patch2);
    }
}

#[cfg(not(feature = "c-api"))]
mod ffi_disabled_tests {
    #[test]
    fn test_ffi_feature_disabled() {
        // This test runs when c-api feature is disabled
        // Just ensures the test file compiles
        println!("FFI tests are disabled - c-api feature not enabled");
    }
}