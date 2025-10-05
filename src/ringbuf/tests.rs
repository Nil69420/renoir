//! Tests for ring buffer implementations

#[cfg(test)]
mod tests {
    use crate::ringbuf::basic::*;
    use crate::ringbuf::sequenced::*;

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

        // Fill buffer
        for i in 0..4 {
            producer.try_push(i).unwrap();
        }

        // Consume half
        for i in 0..2 {
            assert_eq!(consumer.try_pop().unwrap(), i);
        }

        // Add more (should wrap around)
        producer.try_push(4).unwrap();
        producer.try_push(5).unwrap();

        // Consume remaining
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
