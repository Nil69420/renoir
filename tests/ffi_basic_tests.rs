//! FFI (C API) Integration Tests
//!
//! Tests for the C foreign function interface to validate
//! C/C++ integration capabilities

#[cfg(feature = "c-api")]
use renoir::ffi::{
    renoir_buffer_pool_config, renoir_buffer_pool_create, renoir_free_string,
    renoir_manager_create, renoir_manager_destroy, renoir_message_release, renoir_publish,
    renoir_publish_commit, renoir_publish_discard, renoir_publish_reserve, renoir_publisher_create,
    renoir_publisher_destroy, renoir_subscribe_read_into, renoir_subscribe_read_next,
    renoir_subscribe_wait, renoir_subscriber_create, renoir_subscriber_destroy,
    renoir_subscriber_overruns, renoir_topic_list, renoir_topic_list_free,
    renoir_topic_manager_create, renoir_topic_manager_destroy, renoir_topic_register,
    renoir_version_major, renoir_version_minor, renoir_version_patch, renoir_version_string,
    RenoirBufferPoolConfig, RenoirBufferPoolHandle, RenoirErrorCode, RenoirMessageMetadata,
    RenoirPublisherHandle, RenoirReceivedMessage, RenoirRegionConfig, RenoirSequenceNumber,
    RenoirSubscriberHandle, RenoirTopicId, RenoirTopicInfo, RenoirTopicOptions,
};
use std::ffi::CStr;
#[cfg(feature = "c-api")]
use std::ffi::CString;
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
        assert_ne!(
            RenoirErrorCode::InvalidParameter,
            RenoirErrorCode::OutOfMemory
        );
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

        // Version functions must match Cargo.toml (no hardcoded drift)
        let version = env!("CARGO_PKG_VERSION");
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "version string must have 3 components");
        let expected_major: u32 = parts[0].parse().unwrap();
        let expected_minor: u32 = parts[1].parse().unwrap();
        let expected_patch: u32 = parts[2].parse().unwrap();
        assert_eq!(major1, expected_major);
        assert_eq!(minor1, expected_minor);
        assert_eq!(patch1, expected_patch);

        let version_ptr = renoir_version_string();
        assert!(!version_ptr.is_null());
        let version_str = unsafe { CStr::from_ptr(version_ptr) }.to_string_lossy();
        assert_eq!(version_str, version);
        renoir_free_string(version_ptr as *mut _);
    }

    // === Tests migrated from inline modules ===

    #[test]
    fn test_topic_manager_create_and_register() {
        let manager = renoir_topic_manager_create();
        assert!(!manager.is_null());

        let topic_name = CString::new("/test/topic").unwrap();
        let options = RenoirTopicOptions {
            pattern: 0,
            ring_capacity: 16,
            max_payload_size: 1024,
            use_shared_pool: false,
            shared_pool_threshold: 0,
            enable_notifications: false,
        };

        let mut topic_id: RenoirTopicId = 0;
        let result = renoir_topic_register(
            manager,
            topic_name.as_ptr(),
            &options as *const _,
            &mut topic_id as *mut _,
        );

        assert_eq!(result, RenoirErrorCode::Success);
        assert_ne!(topic_id, 0);

        let destroy_result = renoir_topic_manager_destroy(manager);
        assert_eq!(destroy_result, RenoirErrorCode::Success);
    }

    #[test]
    fn test_topic_list_free_path() {
        unsafe {
            let manager = renoir_topic_manager_create();
            assert!(!manager.is_null());

            for name in ["/list/a", "/list/b", "/list/c"] {
                let topic_name = CString::new(name).unwrap();
                let options = RenoirTopicOptions {
                    pattern: 0,
                    ring_capacity: 16,
                    max_payload_size: 1024,
                    use_shared_pool: false,
                    shared_pool_threshold: 0,
                    enable_notifications: false,
                };
                let mut topic_id: RenoirTopicId = 0;
                let result = renoir_topic_register(
                    manager,
                    topic_name.as_ptr(),
                    &options as *const _,
                    &mut topic_id as *mut _,
                );
                assert_eq!(result, RenoirErrorCode::Success);
            }

            let mut topics: *mut RenoirTopicInfo = ptr::null_mut();
            let mut count: usize = 0;
            let result = renoir_topic_list(manager, &mut topics, &mut count);
            assert_eq!(result, RenoirErrorCode::Success);
            assert_eq!(count, 3);
            assert!(!topics.is_null());

            for i in 0..count {
                let name = CStr::from_ptr((*topics.add(i)).name);
                assert!(!name.to_bytes().is_empty());
            }

            renoir_topic_list_free(topics, count);
            renoir_topic_manager_destroy(manager);
        }
    }

    #[test]
    fn test_buffer_pool_config_name_stays_valid() {
        unsafe {
            let manager = renoir_manager_create();
            assert!(!manager.is_null());

            let region_name = CString::new("config_test_region").unwrap();
            let region_config = RenoirRegionConfig {
                name: region_name.as_ptr(),
                size: 1024 * 1024,
                backing_type: 0,
                file_path: ptr::null(),
                create: true,
                permissions: 0o600,
            };
            let mut region: renoir::ffi::RenoirRegionHandle = ptr::null_mut();
            let result = renoir::ffi::renoir_region_create(manager, &region_config, &mut region);
            assert_eq!(result, RenoirErrorCode::Success);

            let pool_name = CString::new("config_test_pool").unwrap();
            let pool_config = RenoirBufferPoolConfig {
                name: pool_name.as_ptr(),
                buffer_size: 4096,
                initial_count: 4,
                max_count: 16,
                alignment: 64,
                pre_allocate: true,
                allocation_timeout_ms: 100,
            };
            let mut pool: RenoirBufferPoolHandle = ptr::null_mut();
            let result =
                renoir_buffer_pool_create(manager, region_name.as_ptr(), &pool_config, &mut pool);
            assert_eq!(result, RenoirErrorCode::Success);

            // Reading the config must hand back a stable name pointer
            let mut out_config = RenoirBufferPoolConfig {
                name: ptr::null(),
                buffer_size: 0,
                initial_count: 0,
                max_count: 0,
                alignment: 0,
                pre_allocate: false,
                allocation_timeout_ms: 0,
            };
            let result = renoir_buffer_pool_config(pool, &mut out_config);
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!out_config.name.is_null());
            let returned_name = CStr::from_ptr(out_config.name).to_string_lossy();
            assert_eq!(returned_name, "config_test_pool");
            assert_eq!(out_config.buffer_size, 4096);

            renoir::ffi::renoir_buffer_pool_destroy(pool);
            renoir::ffi::renoir_region_destroy(manager, region_name.as_ptr());
            renoir_manager_destroy(manager);
        }
    }

    #[test]
    fn test_ffi_publish_subscribe_data_path() {
        unsafe {
            let manager = renoir_topic_manager_create();
            assert!(!manager.is_null());

            let topic_name = CString::new("/ffi/data").unwrap();
            let options = RenoirTopicOptions {
                pattern: 0, // SPSC
                ring_capacity: 16,
                max_payload_size: 4096,
                use_shared_pool: false,
                shared_pool_threshold: 0,
                enable_notifications: true,
            };
            let mut topic_id: RenoirTopicId = 0;
            let result = renoir_topic_register(
                manager,
                topic_name.as_ptr(),
                &options as *const _,
                &mut topic_id as *mut _,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert_ne!(topic_id, 0);

            let mut publisher: RenoirPublisherHandle = ptr::null_mut();
            let pub_opts = renoir::ffi::RenoirPublisherOptions {
                batch_size: 1,
                timeout_ms: 0,
                priority: 0,
                enable_batching: false,
            };
            let result = renoir_publisher_create(
                manager,
                topic_name.as_ptr(),
                &pub_opts as *const _,
                &mut publisher,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!publisher.is_null());

            let mut subscriber: RenoirSubscriberHandle = ptr::null_mut();
            let sub_opts = renoir::ffi::RenoirSubscriberOptions {
                mode: 1, // non-blocking
                batch_size: 1,
                timeout_ms: 0,
                queue_depth: 16,
                enable_filtering: false,
            };
            let result = renoir_subscriber_create(
                manager,
                topic_name.as_ptr(),
                &sub_opts as *const _,
                &mut subscriber,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!subscriber.is_null());

            // Publish a payload larger than a MessageDescriptor (>16 bytes)
            let payload = b"ffi data path round trip payload".to_vec();
            let mut seq: RenoirSequenceNumber = 0;
            let result = renoir_publish(
                publisher,
                payload.as_ptr(),
                payload.len(),
                ptr::null(),
                &mut seq,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(seq > 0);

            // Read it back and verify content
            let mut msg = RenoirReceivedMessage {
                payload_ptr: ptr::null(),
                payload_len: 0,
                metadata: renoir::ffi::RenoirMessageMetadata {
                    timestamp_ns: 0,
                    sequence_number: 0,
                    source_id: 0,
                    message_type: 0,
                    flags: 0,
                },
                handle: ptr::null_mut(),
            };
            let result = renoir_subscribe_read_next(subscriber, &mut msg, 0);
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!msg.payload_ptr.is_null());
            assert_eq!(msg.payload_len, payload.len());
            let received = std::slice::from_raw_parts(msg.payload_ptr, msg.payload_len);
            assert_eq!(received, payload.as_slice());

            // Second read must be empty
            let mut empty_msg = RenoirReceivedMessage {
                payload_ptr: ptr::null(),
                payload_len: 0,
                metadata: renoir::ffi::RenoirMessageMetadata {
                    timestamp_ns: 0,
                    sequence_number: 0,
                    source_id: 0,
                    message_type: 0,
                    flags: 0,
                },
                handle: ptr::null_mut(),
            };
            let result = renoir_subscribe_read_next(subscriber, &mut empty_msg, 0);
            assert_eq!(result, RenoirErrorCode::BufferEmpty);

            renoir_message_release(msg.handle);
            renoir_subscriber_destroy(subscriber);
            renoir_publisher_destroy(publisher);
            renoir_topic_manager_destroy(manager);
        }
    }

    #[test]
    fn test_ffi_broadcast_zero_copy_round_trip() {
        unsafe {
            let manager = renoir_topic_manager_create();
            let topic_name = CString::new(format!("/ffi/zc_{}", std::process::id())).unwrap();
            let options = RenoirTopicOptions {
                pattern: 1,
                ring_capacity: 1024,
                max_payload_size: 4096,
                use_shared_pool: false,
                shared_pool_threshold: 0,
                enable_notifications: false,
            };
            let mut topic_id: RenoirTopicId = 0;
            assert_eq!(
                renoir_topic_register(manager, topic_name.as_ptr(), &options, &mut topic_id),
                RenoirErrorCode::Success
            );
            let pub_opts = renoir::ffi::RenoirPublisherOptions {
                batch_size: 1,
                timeout_ms: 0,
                priority: 0,
                enable_batching: false,
            };
            let sub_opts = renoir::ffi::RenoirSubscriberOptions {
                mode: 1,
                batch_size: 1,
                timeout_ms: 0,
                queue_depth: 16,
                enable_filtering: false,
            };
            let mut publisher: RenoirPublisherHandle = ptr::null_mut();
            assert_eq!(
                renoir_publisher_create(manager, topic_name.as_ptr(), &pub_opts, &mut publisher),
                RenoirErrorCode::Success
            );
            let mut sub_a: RenoirSubscriberHandle = ptr::null_mut();
            let mut sub_b: RenoirSubscriberHandle = ptr::null_mut();
            assert_eq!(
                renoir_subscriber_create(manager, topic_name.as_ptr(), &sub_opts, &mut sub_a),
                RenoirErrorCode::Success
            );
            assert_eq!(
                renoir_subscriber_create(manager, topic_name.as_ptr(), &sub_opts, &mut sub_b),
                RenoirErrorCode::Success
            );

            let mut buf = vec![0u8; 4096];
            let mut out_len = 0usize;
            assert_eq!(
                renoir_subscribe_read_into(
                    sub_a,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut out_len,
                    ptr::null_mut()
                ),
                RenoirErrorCode::BufferEmpty
            );

            for i in 1..=5u8 {
                let mut slot: *mut u8 = ptr::null_mut();
                let mut handle = ptr::null_mut();
                assert_eq!(
                    renoir_publish_reserve(publisher, 100, &mut slot, &mut handle),
                    RenoirErrorCode::Success
                );
                assert!(!slot.is_null());
                assert_eq!(slot as usize % 64, 0);
                let mut second: *mut u8 = ptr::null_mut();
                let mut second_handle = ptr::null_mut();
                assert_eq!(
                    renoir_publish_reserve(publisher, 100, &mut second, &mut second_handle),
                    RenoirErrorCode::InvalidParameter
                );
                std::slice::from_raw_parts_mut(slot, 100).fill(i);
                assert_eq!(
                    renoir_subscribe_read_into(
                        sub_a,
                        buf.as_mut_ptr(),
                        buf.len(),
                        &mut out_len,
                        ptr::null_mut()
                    ),
                    RenoirErrorCode::BufferEmpty
                );
                let mut seq: RenoirSequenceNumber = 0;
                assert_eq!(
                    renoir_publish_commit(publisher, handle, 100, ptr::null(), &mut seq),
                    RenoirErrorCode::Success
                );
                assert_eq!(seq, i as u64);

                for sub in [sub_a, sub_b] {
                    let mut meta: RenoirMessageMetadata = std::mem::zeroed();
                    assert_eq!(
                        renoir_subscribe_read_into(
                            sub,
                            buf.as_mut_ptr(),
                            buf.len(),
                            &mut out_len,
                            &mut meta
                        ),
                        RenoirErrorCode::Success
                    );
                    assert_eq!(out_len, 100);
                    assert!(buf[..100].iter().all(|b| *b == i));
                    assert_eq!(meta.sequence_number, i as u64);
                    assert!(meta.timestamp_ns > 0);
                }
            }

            let mut slot: *mut u8 = ptr::null_mut();
            let mut handle = ptr::null_mut();
            assert_eq!(
                renoir_publish_reserve(publisher, 100, &mut slot, &mut handle),
                RenoirErrorCode::Success
            );
            assert_eq!(
                renoir_publish_discard(publisher, handle),
                RenoirErrorCode::Success
            );
            assert_eq!(
                renoir_publish_reserve(publisher, 100, &mut slot, &mut handle),
                RenoirErrorCode::Success
            );
            std::slice::from_raw_parts_mut(slot, 100).fill(9);
            assert_eq!(
                renoir_publish_commit(publisher, handle, 100, ptr::null(), ptr::null_mut()),
                RenoirErrorCode::Success
            );
            let mut tiny = [0u8; 10];
            assert_eq!(
                renoir_subscribe_read_into(
                    sub_a,
                    tiny.as_mut_ptr(),
                    tiny.len(),
                    &mut out_len,
                    ptr::null_mut()
                ),
                RenoirErrorCode::InsufficientSpace
            );
            assert_eq!(out_len, 100);
            assert_eq!(
                renoir_subscribe_read_into(
                    sub_a,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut out_len,
                    ptr::null_mut()
                ),
                RenoirErrorCode::Success
            );
            assert!(buf[..100].iter().all(|b| *b == 9));

            for _ in 0..600 {
                let mut slot: *mut u8 = ptr::null_mut();
                let mut handle = ptr::null_mut();
                assert_eq!(
                    renoir_publish_reserve(publisher, 4096, &mut slot, &mut handle),
                    RenoirErrorCode::Success
                );
                assert_eq!(
                    renoir_publish_commit(publisher, handle, 4096, ptr::null(), ptr::null_mut()),
                    RenoirErrorCode::Success
                );
            }
            let mut overruns = 0u64;
            assert_eq!(
                renoir_subscriber_overruns(sub_b, &mut overruns),
                RenoirErrorCode::Success
            );
            assert_eq!(overruns, 0);
            let mut drained = 0u64;
            while renoir_subscribe_read_into(
                sub_b,
                buf.as_mut_ptr(),
                buf.len(),
                &mut out_len,
                ptr::null_mut(),
            ) == RenoirErrorCode::Success
            {
                drained += 1;
            }
            assert_eq!(
                renoir_subscriber_overruns(sub_b, &mut overruns),
                RenoirErrorCode::Success
            );
            assert!(overruns > 0);
            assert_eq!(drained + overruns, 601);

            renoir_subscriber_destroy(sub_a);
            renoir_subscriber_destroy(sub_b);
            renoir_publisher_destroy(publisher);
            renoir_topic_manager_destroy(manager);
        }
    }

    #[test]
    fn test_ffi_subscribe_wait_blocks_until_publish() {
        {
            let manager = renoir_topic_manager_create();
            let topic_name = CString::new(format!("/ffi/wait_{}", std::process::id())).unwrap();
            let options = RenoirTopicOptions {
                pattern: 1,
                ring_capacity: 256,
                max_payload_size: 256,
                use_shared_pool: false,
                shared_pool_threshold: 0,
                enable_notifications: false,
            };
            let mut topic_id: RenoirTopicId = 0;
            assert_eq!(
                renoir_topic_register(manager, topic_name.as_ptr(), &options, &mut topic_id),
                RenoirErrorCode::Success
            );
            let pub_opts = renoir::ffi::RenoirPublisherOptions {
                batch_size: 1,
                timeout_ms: 0,
                priority: 0,
                enable_batching: false,
            };
            let sub_opts = renoir::ffi::RenoirSubscriberOptions {
                mode: 1,
                batch_size: 1,
                timeout_ms: 0,
                queue_depth: 16,
                enable_filtering: false,
            };
            let mut publisher: RenoirPublisherHandle = ptr::null_mut();
            let mut subscriber: RenoirSubscriberHandle = ptr::null_mut();
            assert_eq!(
                renoir_publisher_create(manager, topic_name.as_ptr(), &pub_opts, &mut publisher),
                RenoirErrorCode::Success
            );
            assert_eq!(
                renoir_subscriber_create(manager, topic_name.as_ptr(), &sub_opts, &mut subscriber),
                RenoirErrorCode::Success
            );

            let start = std::time::Instant::now();
            assert_eq!(
                renoir_subscribe_wait(subscriber, 60),
                RenoirErrorCode::BufferEmpty
            );
            assert!(start.elapsed() >= std::time::Duration::from_millis(55));

            let pub_addr = publisher as usize;
            let helper = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let payload = [7u8; 16];
                let mut seq: RenoirSequenceNumber = 0;
                renoir_publish(
                    pub_addr as RenoirPublisherHandle,
                    payload.as_ptr(),
                    payload.len(),
                    ptr::null(),
                    &mut seq,
                )
            });
            let start = std::time::Instant::now();
            assert_eq!(
                renoir_subscribe_wait(subscriber, 5000),
                RenoirErrorCode::Success
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            assert_eq!(helper.join().unwrap(), RenoirErrorCode::Success);

            let mut buf = [0u8; 64];
            let mut out_len = 0usize;
            assert_eq!(
                renoir_subscribe_read_into(
                    subscriber,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut out_len,
                    ptr::null_mut()
                ),
                RenoirErrorCode::Success
            );
            assert_eq!(out_len, 16);

            renoir_subscriber_destroy(subscriber);
            renoir_publisher_destroy(publisher);
            renoir_topic_manager_destroy(manager);
        }
    }

    #[test]
    fn test_ffi_destroy_marks_region_clean_oneshot_survives() {
        unsafe {
            // First session: register a topic, publish one message, then
            // release everything through the FFI destroy paths. C and
            // Python callers never hit Rust Drop, so the CLEAN marker must
            // be written by renoir_topic_manager_destroy; a later opener
            // must then preserve the published message instead of wiping it
            // (the one-shot-publisher scenario from analysis §11.5).
            let manager = renoir_topic_manager_create();
            assert!(!manager.is_null());

            let topic_name = CString::new("/ffi/oneshot").unwrap();
            let options = RenoirTopicOptions {
                pattern: 0, // SPSC
                ring_capacity: 16,
                max_payload_size: 4096,
                use_shared_pool: false,
                shared_pool_threshold: 0,
                enable_notifications: true,
            };
            let mut topic_id: RenoirTopicId = 0;
            let result = renoir_topic_register(
                manager,
                topic_name.as_ptr(),
                &options as *const _,
                &mut topic_id as *mut _,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert_ne!(topic_id, 0);

            let mut publisher: RenoirPublisherHandle = ptr::null_mut();
            let pub_opts = renoir::ffi::RenoirPublisherOptions {
                batch_size: 1,
                timeout_ms: 0,
                priority: 0,
                enable_batching: false,
            };
            let result = renoir_publisher_create(
                manager,
                topic_name.as_ptr(),
                &pub_opts as *const _,
                &mut publisher,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!publisher.is_null());

            let payload = b"ffi one-shot survive payload".to_vec();
            let mut seq: RenoirSequenceNumber = 0;
            let result = renoir_publish(
                publisher,
                payload.as_ptr(),
                payload.len(),
                ptr::null(),
                &mut seq,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(seq > 0);

            renoir_publisher_destroy(publisher);
            renoir_topic_manager_destroy(manager);

            // Second session in the same process: the previous session
            // closed cleanly through the FFI destroy path, so a fresh
            // attach must see the one-shot message.
            let manager2 = renoir_topic_manager_create();
            assert!(!manager2.is_null());

            let mut topic_id2: RenoirTopicId = 0;
            let result = renoir_topic_register(
                manager2,
                topic_name.as_ptr(),
                &options as *const _,
                &mut topic_id2 as *mut _,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert_ne!(topic_id2, 0);

            let mut subscriber: RenoirSubscriberHandle = ptr::null_mut();
            let sub_opts = renoir::ffi::RenoirSubscriberOptions {
                mode: 1, // non-blocking
                batch_size: 1,
                timeout_ms: 0,
                queue_depth: 16,
                enable_filtering: false,
            };
            let result = renoir_subscriber_create(
                manager2,
                topic_name.as_ptr(),
                &sub_opts as *const _,
                &mut subscriber,
            );
            assert_eq!(result, RenoirErrorCode::Success);
            assert!(!subscriber.is_null());

            let mut msg = RenoirReceivedMessage {
                payload_ptr: ptr::null(),
                payload_len: 0,
                metadata: renoir::ffi::RenoirMessageMetadata {
                    timestamp_ns: 0,
                    sequence_number: 0,
                    source_id: 0,
                    message_type: 0,
                    flags: 0,
                },
                handle: ptr::null_mut(),
            };
            let result = renoir_subscribe_read_next(subscriber, &mut msg, 0);
            assert_eq!(
                result,
                RenoirErrorCode::Success,
                "one-shot FFI session's message must survive the manager destroy"
            );
            assert!(!msg.payload_ptr.is_null());
            assert_eq!(msg.payload_len, payload.len());
            let received = std::slice::from_raw_parts(msg.payload_ptr, msg.payload_len);
            assert_eq!(received, payload.as_slice());

            renoir_message_release(msg.handle);
            renoir_subscriber_destroy(subscriber);
            renoir_topic_manager_destroy(manager2);
        }
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
