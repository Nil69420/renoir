//! Tests for topic rings

#[cfg(test)]
mod tests {
    use renoir::{
        topic::{Message, TopicStats},
        topic_rings::{SPSCTopicRing, MPMCTopicRing},
    };
    use std::sync::Arc;

    #[test]
    fn test_spsc_ring_creation() {
        let stats = Arc::new(TopicStats::default());
        let ring = SPSCTopicRing::new(1024, stats).unwrap();
        assert_eq!(ring.capacity(), 1024);
        assert_eq!(ring.utilization(), 0.0);
    }

    #[test]
    fn test_spsc_publish_consume() {
        let stats = Arc::new(TopicStats::default());
        let ring = SPSCTopicRing::new(4096, stats).unwrap();
        
        let message = Message::new_inline(1, 1, b"Hello, Topic!".to_vec());
        
        // Publish message
        ring.try_publish(&message).unwrap();
        assert!(ring.utilization() > 0.0);
        
        // Consume message
        let consumed = ring.try_consume().unwrap();
        assert!(consumed.is_some());
        
        let consumed_msg = consumed.unwrap();
        let topic_id = consumed_msg.header.topic_id;
        let sequence = consumed_msg.header.sequence;
        assert_eq!(topic_id, 1);
        assert_eq!(sequence, 1);
    }

    #[test]
    fn test_mpmc_ring_creation() {
        let stats = Arc::new(TopicStats::default());
        let ring = MPMCTopicRing::new(2048, stats).unwrap();
        assert_eq!(ring.capacity(), 2048);
    }

    #[test]
    fn test_mpmc_concurrent_access() {
        let stats = Arc::new(TopicStats::default());
        let ring = Arc::new(MPMCTopicRing::new(8192, stats).unwrap());
        
        let ring_clone = ring.clone();
        let producer = std::thread::spawn(move || {
            for i in 0..10 {
                let message = Message::new_inline(1, i, format!("Message {}", i).into_bytes());
                ring_clone.try_publish(&message).unwrap();
            }
        });
        
        let ring_clone = ring.clone();
        let consumer = std::thread::spawn(move || {
            let mut count = 0;
            while count < 10 {
                if let Some(_msg) = ring_clone.try_consume().unwrap() {
                    count += 1;
                }
            }
            count
        });
        
        producer.join().unwrap();
        let consumed_count = consumer.join().unwrap();
        assert_eq!(consumed_count, 10);
    }
}