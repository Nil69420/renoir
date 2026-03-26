//! Tests for topic rings

#[cfg(test)]
mod tests {
    use renoir::{
        ringbuf::basic::RingBuffer,
        ringbuf::sequenced::SequencedRingBuffer,
        topic::{Message, TopicStats},
        topic_rings::{MPMCTopicRing, SPSCTopicRing},
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

    // === Tests migrated from inline modules ===

    #[test]
    fn test_ring_buffer_basic() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        assert!(buffer.is_empty());
        assert_eq!(buffer.capacity(), 4);

        producer.try_push(1).unwrap();
        producer.try_push(2).unwrap();

        assert_eq!(buffer.len(), 2);
        assert!(!buffer.is_empty());

        assert_eq!(consumer.try_pop().unwrap(), 1);
        assert_eq!(consumer.try_pop().unwrap(), 2);

        assert!(buffer.is_empty());
    }

    #[test]
    fn test_ring_buffer_full() {
        let buffer: RingBuffer<i32> = RingBuffer::new(2).unwrap();
        let producer = buffer.producer();

        producer.try_push(1).unwrap();
        producer.try_push(2).unwrap();

        assert!(buffer.is_full());
        assert!(producer.try_push(3).is_err());
    }

    #[test]
    fn test_ring_buffer_wrap_around() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        for i in 0..4 {
            producer.try_push(i).unwrap();
        }

        for i in 0..2 {
            assert_eq!(consumer.try_pop().unwrap(), i);
        }

        producer.try_push(4).unwrap();
        producer.try_push(5).unwrap();

        assert_eq!(consumer.try_pop().unwrap(), 2);
        assert_eq!(consumer.try_pop().unwrap(), 3);
        assert_eq!(consumer.try_pop().unwrap(), 4);
        assert_eq!(consumer.try_pop().unwrap(), 5);
    }

    #[test]
    fn test_sequenced_ring_buffer() {
        let buffer: SequencedRingBuffer<i32> = SequencedRingBuffer::new(4).unwrap();

        let guard = buffer.try_claim().unwrap();
        guard.write(42);

        assert_eq!(buffer.try_read().unwrap(), 42);
    }
}
