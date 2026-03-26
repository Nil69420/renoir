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

    // === try_peek tests ===

    #[test]
    fn test_spsc_peek_returns_head_without_consuming() {
        let stats = Arc::new(TopicStats::default());
        let ring = SPSCTopicRing::new(4096, stats).unwrap();

        let msg = Message::new_inline(1, 1, b"peek me".to_vec());
        ring.try_publish(&msg).unwrap();

        // First peek returns the head message
        let peeked1 = ring.try_peek().unwrap();
        assert!(peeked1.is_some());
        let peeked1 = peeked1.unwrap();
        let seq1 = peeked1.header.sequence;
        assert_eq!(seq1, 1);

        // Second peek returns the same message (not consumed)
        let peeked2 = ring.try_peek().unwrap();
        assert!(peeked2.is_some());
        let peeked2 = peeked2.unwrap();
        let seq2 = peeked2.header.sequence;
        assert_eq!(seq2, seq1);
    }

    #[test]
    fn test_spsc_peek_then_consume_advances() {
        let stats = Arc::new(TopicStats::default());
        let ring = SPSCTopicRing::new(4096, stats).unwrap();

        let msg1 = Message::new_inline(1, 1, b"first".to_vec());
        let msg2 = Message::new_inline(1, 2, b"second".to_vec());
        ring.try_publish(&msg1).unwrap();
        ring.try_publish(&msg2).unwrap();

        // Peek sees first message
        let peeked = ring.try_peek().unwrap().unwrap();
        let peeked_seq = peeked.header.sequence;
        assert_eq!(peeked_seq, 1);

        // Consume removes the first message
        let consumed = ring.try_consume().unwrap().unwrap();
        let consumed_seq = consumed.header.sequence;
        assert_eq!(consumed_seq, 1);

        // Peek now sees the second message
        let peeked_next = ring.try_peek().unwrap().unwrap();
        let next_seq = peeked_next.header.sequence;
        assert_eq!(next_seq, 2);
    }

    #[test]
    fn test_mpmc_peek_returns_head_without_consuming() {
        let stats = Arc::new(TopicStats::default());
        let ring = MPMCTopicRing::new(4096, stats).unwrap();

        let msg = Message::new_inline(1, 1, b"peek me mpmc".to_vec());
        ring.try_publish(&msg).unwrap();

        // First peek returns the head message
        let peeked1 = ring.try_peek().unwrap();
        assert!(peeked1.is_some());
        let peeked1 = peeked1.unwrap();
        let seq1 = peeked1.header.sequence;
        assert_eq!(seq1, 1);

        // Second peek returns the same message (not consumed)
        let peeked2 = ring.try_peek().unwrap();
        assert!(peeked2.is_some());
        let peeked2 = peeked2.unwrap();
        let seq2 = peeked2.header.sequence;
        assert_eq!(seq2, seq1);
    }

    #[test]
    fn test_mpmc_peek_then_consume_advances() {
        let stats = Arc::new(TopicStats::default());
        let ring = MPMCTopicRing::new(4096, stats).unwrap();

        let msg1 = Message::new_inline(1, 1, b"first".to_vec());
        let msg2 = Message::new_inline(1, 2, b"second".to_vec());
        ring.try_publish(&msg1).unwrap();
        ring.try_publish(&msg2).unwrap();

        // Peek sees first message
        let peeked = ring.try_peek().unwrap().unwrap();
        let peeked_seq = peeked.header.sequence;
        assert_eq!(peeked_seq, 1);

        // Consume removes the first message
        let consumed = ring.try_consume().unwrap().unwrap();
        let consumed_seq = consumed.header.sequence;
        assert_eq!(consumed_seq, 1);

        // Peek now sees the second message
        let peeked_next = ring.try_peek().unwrap().unwrap();
        let next_seq = peeked_next.header.sequence;
        assert_eq!(next_seq, 2);
    }

    #[test]
    fn test_peek_empty_ring_returns_none() {
        let stats = Arc::new(TopicStats::default());
        let spsc_ring = SPSCTopicRing::new(4096, stats.clone()).unwrap();
        let mpmc_ring = MPMCTopicRing::new(4096, stats).unwrap();

        assert!(spsc_ring.try_peek().unwrap().is_none());
        assert!(mpmc_ring.try_peek().unwrap().is_none());
    }
}
