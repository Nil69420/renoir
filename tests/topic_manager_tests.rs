//! Tests for topic manager components

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    
    use renoir::{
        topic_manager_modules::{TopicManager, TopicManagerStats},
        topic::{TopicConfig, TopicPattern, TopicQoS, Reliability},
    };

    #[test]
    fn test_topic_manager_creation() {
        let manager = TopicManager::new().unwrap();
        let stats = manager.manager_stats();
        assert_eq!(stats.topics_created.load(Ordering::Relaxed), 0);
        assert_eq!(manager.topic_count(), 0);
    }

    #[test]  
    fn test_topic_creation_and_messaging() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "test_topic".to_string(),
            pattern: TopicPattern::SPSC,
            ring_capacity: 1024,
            max_payload_size: 1024,
            use_shared_pool: false,
            shared_pool_threshold: 4096,
            enable_notifications: true,
            qos: TopicQoS {
                durability: 1,
                reliability: Reliability::BestEffort,
                history_depth: 10,
            },
        };

        // Create topic
        let topic_id = manager.create_topic(config).unwrap();
        assert!(topic_id > 0);
        assert_eq!(manager.topic_count(), 1);
        assert!(manager.has_topic("test_topic"));

        // Create publisher and subscriber
        let publisher = manager.create_publisher("test_topic").unwrap();
        let subscriber = manager.create_subscriber("test_topic").unwrap();

        assert_eq!(publisher.topic_id, topic_id);
        assert_eq!(subscriber.topic_id, topic_id);
        assert_eq!(publisher.topic_name(), "test_topic");
        assert_eq!(subscriber.topic_name(), "test_topic");

        // Publish a message
        let payload = b"Hello, Renoir Topics!".to_vec();
        publisher.publish(payload.clone()).unwrap();

        // Subscribe to the message
        let received = subscriber.subscribe().unwrap();
        assert!(received.is_some());

        let message = received.unwrap();
        let header_topic_id = message.header.topic_id;
        assert_eq!(header_topic_id, topic_id);
        
        match message.payload {
            renoir::topic::MessagePayload::Inline(data) => {
                assert_eq!(data, payload);
            }
            _ => panic!("Expected inline payload"),
        }

        // Verify statistics
        let stats = subscriber.stats();
        assert_eq!(stats.messages_published.load(Ordering::Relaxed), 1);
        assert_eq!(stats.messages_consumed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_spsc_pattern_enforcement() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "spsc_topic".to_string(),
            pattern: TopicPattern::SPSC,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        // First publisher should succeed
        let _pub1 = manager.create_publisher("spsc_topic").unwrap();
        
        // Second publisher should fail
        let pub2_result = manager.create_publisher("spsc_topic");
        assert!(pub2_result.is_err());

        // First subscriber should succeed
        let _sub1 = manager.create_subscriber("spsc_topic").unwrap();
        
        // Second subscriber should fail
        let sub2_result = manager.create_subscriber("spsc_topic");
        assert!(sub2_result.is_err());
    }

    #[test]
    fn test_mpmc_pattern_multiple_handles() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "mpmc_topic".to_string(),
            pattern: TopicPattern::MPMC,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        // Multiple publishers should succeed
        let _pub1 = manager.create_publisher("mpmc_topic").unwrap();
        let _pub2 = manager.create_publisher("mpmc_topic").unwrap();
        
        // Multiple subscribers should succeed
        let _sub1 = manager.create_subscriber("mpmc_topic").unwrap();
        let _sub2 = manager.create_subscriber("mpmc_topic").unwrap();
    }

    #[test]
    fn test_large_message_with_shared_pools() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "large_msg_topic".to_string(),
            pattern: TopicPattern::SPSC,
            use_shared_pool: true,
            shared_pool_threshold: 1024,
            max_payload_size: 8192,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        let publisher = manager.create_publisher("large_msg_topic").unwrap();
        let subscriber = manager.create_subscriber("large_msg_topic").unwrap();

        // Publish a large message (will use shared pool)
        let large_payload = vec![0x42u8; 2048];
        publisher.publish(large_payload.clone()).unwrap();

        // Subscribe and verify
        let received = subscriber.subscribe().unwrap();
        assert!(received.is_some());

        let message = received.unwrap();
        match message.payload {
            renoir::topic::MessagePayload::Descriptor(desc) => {
                // Verify descriptor properties
                assert_eq!(desc.payload_size, 2048);
                assert!(desc.pool_id > 0);
            }
            _ => panic!("Expected descriptor payload for large message"),
        }
    }

    #[test]
    fn test_topic_removal() {
        let manager = TopicManager::new().unwrap();

        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let topic_name = format!("removable_topic_{}_{}", 
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        
        let config = TopicConfig {
            name: topic_name.clone(),
            ..Default::default()
        };

        manager.create_topic(config).unwrap();
        assert_eq!(manager.topic_count(), 1);

        // Should be able to remove topic with no publishers/subscribers
        manager.remove_topic(&topic_name).unwrap();
        assert_eq!(manager.topic_count(), 0);
        assert!(!manager.has_topic(&topic_name));

        // Recreate topic
        let config = TopicConfig {
            name: topic_name.clone(),
            ..Default::default()
        };
        manager.create_topic(config).unwrap();
        let _publisher = manager.create_publisher(&topic_name).unwrap();
        
        // Should not be able to remove topic with active publisher
        let result = manager.remove_topic(&topic_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_publisher_subscriber_features() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "feature_test_topic".to_string(),
            pattern: TopicPattern::SPSC,
            ring_capacity: 10,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        let publisher = manager.create_publisher("feature_test_topic").unwrap();
        let subscriber = manager.create_subscriber("feature_test_topic").unwrap();

        // Test publisher features
        assert!(!publisher.is_full());
        assert_eq!(publisher.pending_messages(), 0);
        assert_eq!(publisher.capacity(), 10);
        assert!(publisher.has_subscribers());

        // Test subscriber features
        assert!(subscriber.is_empty());
        assert_eq!(subscriber.pending_messages(), 0);
        assert_eq!(subscriber.last_sequence(), 0);
        assert!(!subscriber.has_new_messages());
        assert!(subscriber.has_publishers());

        // Publish and test
        publisher.publish(b"test1".to_vec()).unwrap();
        publisher.publish(b"test2".to_vec()).unwrap();

        assert_eq!(publisher.pending_messages(), 2);
        assert_eq!(subscriber.pending_messages(), 2);
        assert!(!subscriber.is_empty());
        assert!(subscriber.has_new_messages());

        // Consume one message
        let msg1 = subscriber.subscribe().unwrap().unwrap();
        assert_eq!(subscriber.last_sequence(), msg1.header.sequence as usize);
        assert_eq!(subscriber.pending_messages(), 1);

        // Drain remaining messages
        let remaining = subscriber.drain_messages().unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(subscriber.is_empty());
    }

    #[test]
    fn test_payload_size_validation() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "size_test_topic".to_string(),
            max_payload_size: 100,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();
        let publisher = manager.create_publisher("size_test_topic").unwrap();

        // Normal size should work
        let normal_payload = vec![0u8; 50];
        assert!(publisher.publish(normal_payload).is_ok());

        // Oversized payload should fail
        let oversized_payload = vec![0u8; 150];
        assert!(publisher.publish(oversized_payload).is_err());
    }

    #[test]
    fn test_try_publish() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "try_publish_topic".to_string(),
            ring_capacity: 2, // Very small capacity
            ..Default::default()
        };

        manager.create_topic(config).unwrap();
        let publisher = manager.create_publisher("try_publish_topic").unwrap();

        // Fill the ring buffer
        assert!(publisher.try_publish(b"msg1".to_vec()).unwrap());
        assert!(publisher.try_publish(b"msg2".to_vec()).unwrap());

        // Should be full now
        assert!(publisher.is_full());
        
        // Try to publish when full should return false
        assert!(!publisher.try_publish(b"msg3".to_vec()).unwrap());
    }

    #[test]
    fn test_topic_manager_stats() {
        let stats = TopicManagerStats::new();
        
        assert_eq!(stats.active_topics(), 0);
        assert_eq!(stats.total_published(), 0);
        assert_eq!(stats.total_consumed(), 0);
        assert_eq!(stats.throughput_efficiency(), 1.0);
        assert_eq!(stats.peak_rate(), 0);

        // Simulate some activity
        stats.topics_created.store(5, Ordering::Relaxed);
        stats.topics_removed.store(2, Ordering::Relaxed);
        stats.record_publish();
        stats.record_publish();
        stats.record_consume();

        assert_eq!(stats.active_topics(), 3);
        assert_eq!(stats.total_published(), 2);
        assert_eq!(stats.total_consumed(), 1);
        assert_eq!(stats.throughput_efficiency(), 0.5);

        stats.update_peak_rate(100);
        assert_eq!(stats.peak_rate(), 100);

        stats.update_peak_rate(50); // Should not update
        assert_eq!(stats.peak_rate(), 100);

        stats.update_peak_rate(150); // Should update
        assert_eq!(stats.peak_rate(), 150);

        // Test reset
        stats.reset();
        assert_eq!(stats.active_topics(), 0);
        assert_eq!(stats.total_published(), 0);
        assert_eq!(stats.peak_rate(), 0);
    }

    #[test]
    fn test_topic_list() {
        let manager = TopicManager::new().unwrap();
        
        let topics = manager.list_topics();
        assert!(topics.is_empty());

        // Create a few topics
        let config1 = TopicConfig {
            name: "topic1".to_string(),
            ..Default::default()
        };
        let config2 = TopicConfig {
            name: "topic2".to_string(),
            pattern: TopicPattern::MPMC,
            ..Default::default()
        };

        let id1 = manager.create_topic(config1).unwrap();
        let id2 = manager.create_topic(config2.clone()).unwrap();

        let topics = manager.list_topics();
        assert_eq!(topics.len(), 2);

        // Find topic1 in the list
        let topic1_entry = topics.iter().find(|(name, _, _)| name == "topic1").unwrap();
        assert_eq!(topic1_entry.1, id1);
        assert_eq!(topic1_entry.2.pattern, TopicPattern::SPSC);

        // Find topic2 in the list  
        let topic2_entry = topics.iter().find(|(name, _, _)| name == "topic2").unwrap();
        assert_eq!(topic2_entry.1, id2);
        assert_eq!(topic2_entry.2.pattern, TopicPattern::MPMC);
    }
}