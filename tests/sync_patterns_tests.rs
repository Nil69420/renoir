//! Comprehensive tests for synchronization patterns
//!
//! Tests cover SWMR rings, epoch reclamation, sequence consistency,
//! notification patterns, and integration scenarios for robotics use cases.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use renoir::sync::{
    epoch::{EpochManager, EpochReclaim},
    notify::{BatchNotifier, EventNotifier, NotificationGroup, NotifyCondition},
    sequence::{
        next_local_sequence, special, AtomicSequence, GlobalSequenceGenerator, SequenceChecker,
    },
    swmr::SWMRRing,
};

#[cfg(test)]
mod sync_patterns_tests {
    use super::*;

    /// Test: Basic atomic sequence operations and consistency
    #[test]
    fn test_atomic_sequence_consistency() {
        let sequence = Arc::new(AtomicSequence::new());
        let checker = SequenceChecker::new(sequence.clone());

        // Initially empty
        assert_eq!(sequence.load_relaxed(), special::EMPTY);
        assert!(AtomicSequence::is_empty(sequence.load_relaxed()));

        // Claim for writing
        assert!(sequence.try_claim_for_write(special::EMPTY));
        assert_eq!(sequence.load_relaxed(), special::WRITING);
        assert!(AtomicSequence::is_writing(sequence.load_relaxed()));

        // Mark ready
        sequence.mark_ready(special::FIRST_DATA).unwrap();
        assert_eq!(sequence.load_relaxed(), special::FIRST_DATA);
        assert!(AtomicSequence::is_ready(sequence.load_relaxed()));

        // Test consistency checking
        let result = checker.consistent_read(|| 42u32, 5);
        assert_eq!(result.unwrap(), 42);
    }

    /// Test: SWMR ring buffer basic operations
    #[test]
    fn test_swmr_basic_operations() {
        let ring = SWMRRing::<u64>::new(8).unwrap();
        let mut writer = ring.writer();
        let mut reader = ring.reader();

        // Write some data
        let seq1 = writer.try_write(100).unwrap();
        let seq2 = writer.try_write(200).unwrap();
        let seq3 = writer.try_write(300).unwrap();

        assert!(seq2 > seq1);
        assert!(seq3 > seq2);

        // Read data back
        let (data1, read_seq1) = reader.try_read().unwrap().unwrap();
        assert_eq!(data1, 100);
        assert_eq!(read_seq1, seq1);

        let (data2, read_seq2) = reader.try_read().unwrap().unwrap();
        assert_eq!(data2, 200);
        assert_eq!(read_seq2, seq2);

        // Reader position tracking
        assert_eq!(reader.read_position(), 2);
        assert_eq!(reader.last_sequence(), seq2);
    }

    /// Test: SWMR with multiple concurrent readers
    #[test]
    fn test_swmr_multiple_readers() {
        let ring = Arc::new(SWMRRing::<u32>::new(16).unwrap());
        let barrier = Arc::new(Barrier::new(4)); // 1 writer + 3 readers

        let write_count = 50;
        let reader_results = Arc::new(Mutex::new(Vec::new()));

        // Start writer
        let writer_ring = ring.clone();
        let writer_barrier = barrier.clone();
        let writer_handle = thread::spawn(move || {
            let mut writer = writer_ring.writer();
            writer_barrier.wait();

            for i in 0..write_count {
                writer.try_write(i).unwrap();
                thread::sleep(Duration::from_micros(100));
            }
        });

        // Start multiple readers
        let reader_handles: Vec<_> = (0..3)
            .map(|reader_id| {
                let reader_ring = ring.clone();
                let reader_barrier = barrier.clone();
                let results = reader_results.clone();

                thread::spawn(move || {
                    let mut reader = reader_ring.reader();
                    reader_barrier.wait();

                    let mut read_data = Vec::new();
                    let start_time = Instant::now();

                    while start_time.elapsed() < Duration::from_millis(200) {
                        if let Ok(Some((data, seq))) = reader.try_read() {
                            read_data.push((data, seq));
                        }
                        thread::sleep(Duration::from_micros(150));
                    }

                    let mut results = results.lock().unwrap();
                    results.push((reader_id, read_data));
                })
            })
            .collect();

        writer_handle.join().unwrap();
        for handle in reader_handles {
            handle.join().unwrap();
        }

        let results = reader_results.lock().unwrap();

        // Each reader should have read some data independently
        for (reader_id, data) in results.iter() {
            println!("Reader {} read {} items", reader_id, data.len());
            assert!(
                !data.is_empty(),
                "Reader {} should have read some data",
                reader_id
            );

            // Verify sequence ordering within each reader
            for window in data.windows(2) {
                assert!(window[1].1 > window[0].1, "Sequences should be increasing");
            }
        }

        // Readers can read at different rates independently
        let read_counts: Vec<_> = results.iter().map(|(_, data)| data.len()).collect();
        println!("Read counts: {:?}", read_counts);
    }

    /// Test: Epoch-based memory reclamation
    #[test]
    fn test_epoch_reclamation() {
        let manager = EpochManager::new();
        let reclaimed_count = Arc::new(AtomicUsize::new(0));

        // Test object that tracks reclamation
        struct TestReclaim {
            counter: Arc<AtomicUsize>,
        }

        impl EpochReclaim for TestReclaim {
            fn reclaim(&mut self) {
                self.counter.fetch_add(1, Ordering::Relaxed);
            }
        }

        let initial_epoch = manager.current_epoch();

        // Defer some objects for reclamation
        for _ in 0..3 {
            let obj = TestReclaim {
                counter: reclaimed_count.clone(),
            };
            manager.defer_reclaim(obj);
        }

        // No readers active, advance epoch multiple times and reclaim
        manager.advance_epoch();
        manager.advance_epoch();

        let reclaimed = manager.try_reclaim();

        assert_eq!(reclaimed, 3);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 3);

        // Verify epoch advanced
        assert!(manager.current_epoch() > initial_epoch);
    }

    /// Test: Epoch reclamation with active readers
    #[test]
    fn test_epoch_with_readers() {
        let manager = Arc::new(EpochManager::new());
        let reclaimed_count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3)); // 2 readers + main thread

        struct TestObj {
            counter: Arc<AtomicUsize>,
        }

        impl EpochReclaim for TestObj {
            fn reclaim(&mut self) {
                self.counter.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Start readers that will hold epochs
        let reader_handles: Vec<_> = (0..2)
            .map(|_reader_id| {
                let manager = manager.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let reader = manager.register_reader();
                    barrier.wait();

                    // Hold epoch for a while
                    let _epoch = reader.enter();
                    thread::sleep(Duration::from_millis(50));
                    reader.leave();
                })
            })
            .collect();

        barrier.wait(); // Wait for readers to start

        // Defer object while readers are active
        let obj = TestObj {
            counter: reclaimed_count.clone(),
        };
        manager.defer_reclaim(obj);

        // Advance epoch but readers are still active
        manager.advance_epoch();

        // Should not be able to reclaim yet
        let reclaimed = manager.try_reclaim();
        assert_eq!(reclaimed, 0);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 0);

        // Wait for readers to finish
        for handle in reader_handles {
            handle.join().unwrap();
        }

        // Now should be able to reclaim
        let reclaimed = manager.try_reclaim();
        assert_eq!(reclaimed, 1);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 1);
    }

    /// Test: Event notification basic functionality
    #[test]
    fn test_event_notification() {
        let notifier = EventNotifier::new().unwrap();

        // Should start enabled
        assert!(notifier.is_enabled());

        // Basic notify
        notifier.notify().unwrap();
        let stats = notifier.stats();
        assert_eq!(stats.notify_count, 1);

        // Disable and verify no side effects
        notifier.set_enabled(false);
        notifier.notify().unwrap(); // Should still succeed
        assert!(!notifier.is_enabled());

        notifier.set_enabled(true);
        assert!(notifier.is_enabled());
    }

    /// Test: Notification groups and channels
    #[test]
    fn test_notification_groups() {
        let group = NotificationGroup::new().unwrap();

        // Add channels
        let sensor1 = group.add_channel("sensor1").unwrap();
        let sensor2 = group.add_channel("sensor2").unwrap();

        // Verify channels exist
        let names = group.channel_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"sensor1".to_string()));
        assert!(names.contains(&"sensor2".to_string()));

        // Individual channel notifications
        sensor1.notify().unwrap();
        sensor2.notify().unwrap();

        // Group operations
        group.notify_all().unwrap();
        group.notify_channels(&["sensor1"]).unwrap();

        // Stats aggregation
        let agg_stats = group.aggregate_stats();
        assert!(agg_stats.total_notify_count >= 4); // At least 4 notifications
        assert_eq!(agg_stats.total_channels, 3); // 2 named + 1 default
    }

    /// Test: Batched notifications with different conditions
    #[test]
    fn test_batch_notifications() {
        // Test write count condition
        let mut batch_notifier = BatchNotifier::new(NotifyCondition::WriteCount(3)).unwrap();

        assert!(!batch_notifier.record_write(None).unwrap());
        assert!(!batch_notifier.record_write(None).unwrap());
        assert!(batch_notifier.record_write(None).unwrap()); // Third write triggers

        // Test buffer level condition
        batch_notifier.set_condition(NotifyCondition::BufferLevel(10));

        assert!(!batch_notifier.record_write(Some(5)).unwrap());
        assert!(batch_notifier.record_write(Some(15)).unwrap());

        // Test always condition
        batch_notifier.set_condition(NotifyCondition::Always);
        assert!(batch_notifier.record_write(None).unwrap());
        assert!(batch_notifier.record_write(None).unwrap());

        let stats = batch_notifier.batch_stats();
        assert!(stats.total_writes >= 6);
    }

    /// Test: Integration of SWMR with notifications
    #[test]
    fn test_swmr_with_notifications() {
        let ring = Arc::new(SWMRRing::<String>::new(8).unwrap());
        let notifier = Arc::new(EventNotifier::new().unwrap());
        let barrier = Arc::new(Barrier::new(2));

        let received_data = Arc::new(Mutex::new(Vec::new()));

        // Writer thread
        let writer_ring = ring.clone();
        let writer_notifier = notifier.clone();
        let writer_barrier = barrier.clone();
        let write_handle = thread::spawn(move || {
            let mut writer = writer_ring.writer();
            writer_barrier.wait();

            for i in 0..10 {
                let message = format!("Message {}", i);
                writer.try_write(message).unwrap();
                writer_notifier.notify().unwrap();
                thread::sleep(Duration::from_millis(5));
            }
        });

        // Reader thread
        let reader_ring = ring.clone();
        let reader_notifier = notifier.clone();
        let reader_barrier = barrier.clone();
        let reader_data = received_data.clone();
        let read_handle = thread::spawn(move || {
            let mut reader = reader_ring.reader();
            reader_barrier.wait();

            for _ in 0..10 {
                // Wait for notification
                if reader_notifier.wait(Some(1000)).is_ok() {
                    // Try to read available data
                    while let Ok(Some((data, _seq))) = reader.try_read() {
                        let mut received = reader_data.lock().unwrap();
                        received.push(data);
                    }
                }
            }
        });

        write_handle.join().unwrap();
        read_handle.join().unwrap();

        let received = received_data.lock().unwrap();
        println!("Received {} messages", received.len());
        assert!(!received.is_empty());

        // Verify message content
        for (i, msg) in received.iter().enumerate().take(5) {
            assert!(
                msg.starts_with("Message"),
                "Message {} should start with 'Message'",
                i
            );
        }
    }

    /// Test: Sequence number consistency under contention
    #[test]
    fn test_sequence_contention() {
        let global_gen = Arc::new(GlobalSequenceGenerator::new());
        let barrier = Arc::new(Barrier::new(4));
        let sequences = Arc::new(Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let gen = global_gen.clone();
                let barrier = barrier.clone();
                let seqs = sequences.clone();

                thread::spawn(move || {
                    barrier.wait();
                    let mut local_seqs = Vec::new();

                    for _ in 0..100 {
                        local_seqs.push(gen.next());
                    }

                    let mut all_seqs = seqs.lock().unwrap();
                    all_seqs.extend(local_seqs);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let mut all_sequences = sequences.lock().unwrap();
        all_sequences.sort();

        // Should have 400 unique sequence numbers
        assert_eq!(all_sequences.len(), 400);

        // Should be consecutive (no duplicates)
        for window in all_sequences.windows(2) {
            assert_eq!(window[1], window[0] + 1);
        }
    }

    /// Test: SWMR reader lag detection and recovery
    #[test]
    fn test_swmr_reader_lag() {
        let ring = SWMRRing::<u64>::new(4).unwrap(); // Small ring to force lag
        let mut writer = ring.writer();
        let mut reader = ring.reader();

        // Fill ring beyond capacity
        for i in 0..8 {
            writer.try_write(i).unwrap();
        }

        // Reader should detect lag
        assert!(reader.is_too_slow());
        assert!(reader.lag() >= ring.capacity());

        // Skip to latest should recover
        reader.skip_to_latest().unwrap();
        assert!(!reader.is_too_slow());
        assert!(reader.lag() < ring.capacity());

        // Try to read latest data (might be None if no data ready)
        match reader.try_read().unwrap() {
            Some((latest_data, _)) => {
                assert!(latest_data >= 4); // Should be one of the later messages
            }
            None => {
                // This is also acceptable - reader has caught up but no data ready
            }
        }
    }

    /// Test: Performance characteristics (timing-based)
    #[test]
    fn test_performance_characteristics() {
        let ring = SWMRRing::<u64>::new(1024).unwrap();
        let mut writer = ring.writer();
        let mut reader = ring.reader();

        let write_count = 1000;

        // Measure write performance
        let start = Instant::now();
        for i in 0..write_count {
            writer.try_write(i).unwrap();
        }
        let write_time = start.elapsed();

        // Measure read performance
        let start = Instant::now();
        let mut read_count = 0;
        while let Ok(Some(_)) = reader.try_read() {
            read_count += 1;
        }
        let read_time = start.elapsed();

        println!(
            "Write: {} ops in {:?} ({:.0} ops/sec)",
            write_count,
            write_time,
            write_count as f64 / write_time.as_secs_f64()
        );

        println!(
            "Read: {} ops in {:?} ({:.0} ops/sec)",
            read_count,
            read_time,
            read_count as f64 / read_time.as_secs_f64()
        );

        // Basic performance expectations (very loose for CI)
        assert!(write_time.as_millis() < 100, "Writes should be fast");
        assert!(read_time.as_millis() < 100, "Reads should be fast");
        assert_eq!(read_count, write_count);
    }

    /// Test: Thread-local sequence generation performance
    #[test]
    fn test_local_sequence_performance() {
        let iterations = 10000;

        let start = Instant::now();
        for _ in 0..iterations {
            let _seq = next_local_sequence();
        }
        let local_time = start.elapsed();

        let global_gen = GlobalSequenceGenerator::new();
        let start = Instant::now();
        for _ in 0..iterations {
            let _seq = global_gen.next();
        }
        let global_time = start.elapsed();

        println!("Local sequence: {} ops in {:?}", iterations, local_time);
        println!("Global sequence: {} ops in {:?}", iterations, global_time);

        // Both should be reasonably fast (performance may vary by system)
        // The important thing is that both methods work correctly
        assert!(
            local_time.as_millis() < 100,
            "Local sequence should be fast"
        );
        assert!(
            global_time.as_millis() < 100,
            "Global sequence should be fast"
        );
    }

    /// Test: Cross-pattern integration (SWMR + Epoch + Notify)
    #[test]
    fn test_integrated_patterns() {
        let ring = Arc::new(SWMRRing::<Vec<u8>>::new(8).unwrap());
        let epoch_manager = Arc::new(EpochManager::new());
        let notifier = Arc::new(EventNotifier::new().unwrap());
        let barrier = Arc::new(Barrier::new(3));

        let processed_count = Arc::new(AtomicUsize::new(0));

        // Producer thread
        let producer_ring = ring.clone();
        let producer_notifier = notifier.clone();
        let producer_barrier = barrier.clone();
        let producer_handle = thread::spawn(move || {
            let mut writer = producer_ring.writer();
            producer_barrier.wait();

            for i in 0..20 {
                let data = vec![i as u8; 64]; // Simulate sensor data
                writer.try_write(data).unwrap();
                producer_notifier.notify().unwrap();
                thread::sleep(Duration::from_millis(1));
            }
        });

        // Consumer thread with epoch management
        let consumer_ring = ring.clone();
        let consumer_epoch = epoch_manager.clone();
        let consumer_notifier = notifier.clone();
        let consumer_barrier = barrier.clone();
        let consumer_count = processed_count.clone();
        let consumer_handle = thread::spawn(move || {
            let mut reader = consumer_ring.reader();
            let epoch_reader = consumer_epoch.register_reader();
            consumer_barrier.wait();

            for _ in 0..25 {
                // Try to read more than produced
                if consumer_notifier.wait(Some(100)).is_ok() {
                    epoch_reader.with_epoch(|_epoch| {
                        while let Ok(Some((data, seq))) = reader.try_read() {
                            // Simulate processing
                            assert!(!data.is_empty());
                            consumer_count.fetch_add(1, Ordering::Relaxed);
                            epoch_reader.update_sequence(seq);
                        }
                    });
                }
            }
        });

        barrier.wait(); // Start all threads

        producer_handle.join().unwrap();
        consumer_handle.join().unwrap();

        let processed = processed_count.load(Ordering::Relaxed);
        println!("Processed {} items in integrated test", processed);

        // Should have processed some data
        assert!(processed > 0);
        assert!(processed <= 20); // Can't process more than produced

        // Epoch system should have stats
        let epoch_stats = epoch_manager.stats();
        assert_eq!(epoch_stats.total_readers, 0); // Reader unregistered when dropped

        // Notification system should have activity
        let notify_stats = notifier.stats();
        assert!(notify_stats.notify_count >= processed as u64);
    }
}
