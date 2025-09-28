//! Tests for topic components

#[cfg(test)]
mod tests {
    use crate::topic::header::*;
    use crate::topic::message::*;
    use crate::topic::stats::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_message_header_creation() {
        let header = MessageHeader::new(42, 1337, 1024);
        
        // Copy values to locals to avoid packed field alignment issues
        let magic = header.magic;
        let version = header.version;
        let topic_id = header.topic_id;
        let sequence = header.sequence;
        let payload_length = header.payload_length;
        let timestamp = header.timestamp;
        
        assert_eq!(magic, MESSAGE_MAGIC);
        assert_eq!(version, MESSAGE_VERSION);
        assert_eq!(topic_id, 42);
        assert_eq!(sequence, 1337);
        assert_eq!(payload_length, 1024);
        assert!(timestamp > 0);
    }

    #[test]
    fn test_message_header_validation() {
        let mut header = MessageHeader::new(1, 1, 100);
        assert!(header.validate().is_ok());
        
        header.magic = 0xDEADBEEF;
        assert!(header.validate().is_err());
    }

    #[test]
    fn test_message_checksum() {
        let payload = b"Hello, Renoir!";
        let mut header = MessageHeader::new(1, 1, payload.len() as u32);
        
        header.set_checksum(payload);
        assert!(header.verify_checksum(payload));
        assert!(!header.verify_checksum(b"Different data"));
    }

    #[test]
    fn test_message_descriptor_ref_counting() {
        let desc = MessageDescriptor::new(1, 1, 1024);
        assert_eq!(desc.ref_count.load(Ordering::SeqCst), 1);
        
        desc.add_ref();
        assert_eq!(desc.ref_count.load(Ordering::SeqCst), 2);
        
        assert!(!desc.release()); // Still has references
        assert!(desc.release());  // Last reference
    }

    #[test]
    fn test_topic_stats() {
        let stats = TopicStats::default();
        
        let seq1 = stats.record_published(1024);
        let seq2 = stats.record_published(2048);
        
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(stats.messages_published.load(Ordering::Relaxed), 2);
        assert_eq!(stats.avg_message_size.load(Ordering::Relaxed), 1536); // (1024 + 2048) / 2
        
        stats.record_consumed();
        assert_eq!(stats.messages_consumed.load(Ordering::Relaxed), 1);
    }
}