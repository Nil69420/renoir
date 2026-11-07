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

    #[test]
    fn test_mpmc_message_straddling_buffer_end_round_trips() {
        let stats = Arc::new(TopicStats::default());
        let ring = MPMCTopicRing::new(256, stats).unwrap();

        for i in 0..4u32 {
            let payload = format!("straddle-payload-{:02}", i).into_bytes();
            let message = Message::new_inline(1, i as u64, payload.clone());
            ring.try_publish(&message).unwrap();

            let consumed = ring.try_consume().unwrap().expect("message available");
            let sequence = consumed.header.sequence;
            assert_eq!(sequence, i as u64);
            match consumed.payload {
                renoir::topic::MessagePayload::Inline(data) => {
                    assert_eq!(data, payload, "payload content preserved for message {}", i);
                }
                other => panic!("expected inline payload, got {:?}", other),
            }
        }
    }

    #[test]
    fn test_mpmc_concurrent_publish_consume_content() {
        let stats = Arc::new(TopicStats::default());
        let ring = Arc::new(MPMCTopicRing::new(4096, stats).unwrap());

        const MSG_COUNT: usize = 300;
        let producer_ring = ring.clone();
        let producer = std::thread::spawn(move || {
            for i in 0..MSG_COUNT {
                let payload = format!("concurrent-payload-{:04}", i).into_bytes();
                let message = Message::new_inline(1, i as u64, payload);
                loop {
                    if producer_ring.try_publish(&message).is_ok() {
                        break;
                    }
                    std::thread::yield_now();
                }
            }
        });

        let consumer_ring = ring.clone();
        let consumer = std::thread::spawn(move || {
            let mut received = std::collections::HashSet::new();
            while received.len() < MSG_COUNT {
                if let Some(msg) = consumer_ring.try_consume().unwrap() {
                    let expected =
                        format!("concurrent-payload-{:04}", msg.header.sequence as usize);
                    match msg.payload {
                        renoir::topic::MessagePayload::Inline(data) => {
                            assert_eq!(data, expected.into_bytes());
                        }
                        other => panic!("expected inline payload, got {:?}", other),
                    }
                    received.insert(msg.header.sequence);
                } else {
                    std::thread::yield_now();
                }
            }
            received.len()
        });

        producer.join().unwrap();
        let count = consumer.join().unwrap();
        assert_eq!(count, MSG_COUNT);
    }

    #[test]
    fn test_spsc_from_memory_round_trip() {
        let stats = Arc::new(TopicStats::default());
        let capacity = 4096usize;
        let total = SPSCTopicRing::required_memory_size(capacity);
        let layout = std::alloc::Layout::from_size_align(
            total,
            std::mem::align_of::<std::sync::atomic::AtomicUsize>(),
        )
        .unwrap();
        let memory = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!memory.is_null());

        let ring = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total,
                capacity,
                stats,
            )
            .unwrap()
        };

        let message = Message::new_inline(1, 1, b"from-memory spsc".to_vec());
        ring.try_publish(&message).unwrap();
        let consumed = ring.try_consume().unwrap().unwrap();
        let seq = consumed.header.sequence;
        assert_eq!(seq, 1);
        match consumed.payload {
            renoir::topic::MessagePayload::Inline(data) => {
                assert_eq!(data, b"from-memory spsc".to_vec());
            }
            other => panic!("expected inline payload, got {:?}", other),
        }

        drop(ring);
        unsafe { std::alloc::dealloc(memory, layout) };
    }

    #[test]
    fn test_mpmc_from_memory_round_trip() {
        let stats = Arc::new(TopicStats::default());
        let capacity = 4096usize;
        let total = MPMCTopicRing::required_memory_size(capacity);
        let layout = std::alloc::Layout::from_size_align(
            total,
            std::mem::align_of::<std::sync::atomic::AtomicUsize>(),
        )
        .unwrap();
        let memory = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!memory.is_null());

        let ring = unsafe {
            MPMCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total,
                capacity,
                stats,
            )
            .unwrap()
        };

        let message = Message::new_inline(1, 1, b"from-memory mpmc".to_vec());
        ring.try_publish(&message).unwrap();
        let consumed = ring.try_consume().unwrap().unwrap();
        let seq = consumed.header.sequence;
        assert_eq!(seq, 1);
        match consumed.payload {
            renoir::topic::MessagePayload::Inline(data) => {
                assert_eq!(data, b"from-memory mpmc".to_vec());
            }
            other => panic!("expected inline payload, got {:?}", other),
        }

        drop(ring);
        unsafe { std::alloc::dealloc(memory, layout) };
    }

    #[test]
    fn test_from_memory_rejects_undersized_and_misaligned() {
        let stats = Arc::new(TopicStats::default());
        let capacity = 512usize;
        let total = SPSCTopicRing::required_memory_size(capacity);
        let layout = std::alloc::Layout::from_size_align(
            total,
            std::mem::align_of::<std::sync::atomic::AtomicUsize>(),
        )
        .unwrap();
        let memory = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!memory.is_null());

        let undersized = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total - 1,
                capacity,
                stats.clone(),
            )
        };
        assert!(undersized.is_err());

        let initialized = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total,
                capacity,
                stats.clone(),
            )
        };
        assert!(initialized.is_ok());
        drop(initialized.unwrap());

        let mismatched = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total,
                capacity / 2,
                stats.clone(),
            )
        };
        assert!(mismatched.is_err());

        let misaligned = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory.add(1)).unwrap(),
                total - 1,
                capacity,
                stats,
            )
        };
        assert!(misaligned.is_err());

        unsafe { std::alloc::dealloc(memory, layout) };
    }

    #[test]
    fn test_control_info_introspection() {
        let stats = Arc::new(TopicStats::default());
        let capacity = 2048usize;

        // SPSC ring over raw memory; introspection must read live positions
        let total = SPSCTopicRing::required_memory_size(capacity);
        let layout = std::alloc::Layout::from_size_align(
            total,
            std::mem::align_of::<std::sync::atomic::AtomicUsize>(),
        )
        .unwrap();
        let memory = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!memory.is_null());

        let ring = unsafe {
            SPSCTopicRing::from_memory(
                std::ptr::NonNull::new(memory).unwrap(),
                total,
                capacity,
                stats,
            )
            .unwrap()
        };
        let region = unsafe { std::slice::from_raw_parts(memory, total) };

        let message = Message::new_inline(1, 7, b"introspect me".to_vec());
        ring.try_publish(&message).unwrap();

        let info = SPSCTopicRing::read_control_info(region).expect("control info readable");
        assert_eq!(info.capacity, capacity);
        assert!(info.pending_bytes() > 0);
        assert!(info.utilization() > 0.0);
        assert!(info.utilization() <= 1.0);

        let (seq, ts) = SPSCTopicRing::read_pending_header(region).expect("pending header");
        assert_eq!(seq, 7);
        assert!(ts > 0);

        // Consume; pending drops back to zero and no header is readable
        ring.try_consume().unwrap();
        let info = SPSCTopicRing::read_control_info(region).unwrap();
        assert_eq!(info.pending_bytes(), 0);
        assert!(SPSCTopicRing::read_pending_header(region).is_none());

        // A raw region without a valid ring magic must be rejected
        let empty = vec![0u8; total];
        assert!(SPSCTopicRing::read_control_info(&empty).is_none());
        assert!(MPMCTopicRing::read_control_info(&empty).is_none());

        drop(ring);
        unsafe { std::alloc::dealloc(memory, layout) };
    }
}
