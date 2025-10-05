//! Test FFI topic functions directly

#[cfg(test)]
mod tests {
    use crate::ffi::topics::*;
    use crate::ffi::types::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn test_topic_manager_create_and_register() {
        // Create manager
        let manager = renoir_topic_manager_create();
        assert!(!manager.is_null());

        // Register a topic
        let topic_name = CString::new("/test/topic").unwrap();
        let options = RenoirTopicOptions {
            pattern: 0,  // SPSC
            ring_capacity: 10,
            max_payload_size: 1024,
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

        println!("Register result: {:?}", result);
        println!("Topic ID: {}", topic_id);

        assert_eq!(result, RenoirErrorCode::Success);
        assert_ne!(topic_id, 0);

        // Cleanup
        let destroy_result = renoir_topic_manager_destroy(manager);
        assert_eq!(destroy_result, RenoirErrorCode::Success);
    }
}
