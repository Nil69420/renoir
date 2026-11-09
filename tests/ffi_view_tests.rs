#![cfg(feature = "c-api")]

use std::ffi::CString;
use std::ptr;

use renoir::ffi::{
    renoir_publish_commit, renoir_publish_reserve, renoir_publisher_create,
    renoir_publisher_destroy, renoir_subscribe_view, renoir_subscribe_view_release,
    renoir_subscriber_create, renoir_subscriber_destroy, renoir_topic_manager_create,
    renoir_topic_manager_destroy, renoir_topic_register, RenoirErrorCode, RenoirFrameView,
    RenoirPublisherHandle, RenoirSubscriberHandle, RenoirTopicId, RenoirTopicOptions,
};

fn publish(publisher: RenoirPublisherHandle, len: usize, fill: u8) {
    unsafe {
        let mut slot: *mut u8 = ptr::null_mut();
        let mut handle = ptr::null_mut();
        assert_eq!(
            renoir_publish_reserve(publisher, len, &mut slot, &mut handle),
            RenoirErrorCode::Success
        );
        std::slice::from_raw_parts_mut(slot, len).fill(fill);
        assert_eq!(
            renoir_publish_commit(publisher, handle, len, ptr::null(), ptr::null_mut()),
            RenoirErrorCode::Success
        );
    }
}

#[test]
fn view_borrows_slot_bytes_and_release_reports_integrity() {
    unsafe {
        let manager = renoir_topic_manager_create();
        let topic = CString::new(format!("/ffi/view_{}", std::process::id())).unwrap();
        let options = RenoirTopicOptions {
            pattern: 1,
            ring_capacity: 2,
            max_payload_size: 256,
            use_shared_pool: false,
            shared_pool_threshold: 0,
            enable_notifications: false,
        };
        let mut id: RenoirTopicId = 0;
        assert_eq!(
            renoir_topic_register(manager, topic.as_ptr(), &options, &mut id),
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
            renoir_publisher_create(manager, topic.as_ptr(), &pub_opts, &mut publisher),
            RenoirErrorCode::Success
        );
        assert_eq!(
            renoir_subscriber_create(manager, topic.as_ptr(), &sub_opts, &mut subscriber),
            RenoirErrorCode::Success
        );

        let mut view: RenoirFrameView = std::mem::zeroed();
        assert_eq!(
            renoir_subscribe_view(subscriber, &mut view),
            RenoirErrorCode::BufferEmpty
        );
        assert_eq!(
            renoir_subscribe_view(ptr::null_mut(), &mut view),
            RenoirErrorCode::InvalidParameter
        );

        publish(publisher, 100, 0x5A);
        assert_eq!(
            renoir_subscribe_view(subscriber, &mut view),
            RenoirErrorCode::Success
        );
        assert_eq!(view.len, 100);
        assert!(view.timestamp_ns > 0);
        assert!(std::slice::from_raw_parts(view.data, view.len)
            .iter()
            .all(|b| *b == 0x5A));
        assert_eq!(
            renoir_subscribe_view_release(subscriber, &view),
            RenoirErrorCode::Success
        );
        assert_eq!(
            renoir_subscribe_view(subscriber, &mut view),
            RenoirErrorCode::BufferEmpty
        );

        publish(publisher, 100, 1);
        assert_eq!(
            renoir_subscribe_view(subscriber, &mut view),
            RenoirErrorCode::Success
        );
        for fill in 2..200u8 {
            publish(publisher, 100, fill);
        }
        assert_eq!(
            renoir_subscribe_view_release(subscriber, &view),
            RenoirErrorCode::ConcurrencyError
        );
        assert_eq!(
            renoir_subscribe_view_release(subscriber, &view),
            RenoirErrorCode::InvalidParameter
        );

        renoir_subscriber_destroy(subscriber);
        renoir_publisher_destroy(publisher);
        renoir_topic_manager_destroy(manager);
    }
}
