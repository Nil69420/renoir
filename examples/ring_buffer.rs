#!/usr/bin/env rust

//! Ring buffer example demonstrating producer-consumer pattern

use renoir::{
    ringbuf::{RingBuffer, SequencedRingBuffer},
    error::RenoirError,
};
use std::{
    thread,
    time::{Duration, Instant},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
};

fn main() {
    println!("Ring Buffer Producer-Consumer Example");
    println!("====================================");

    // Example 1: Simple ring buffer
    simple_ring_buffer_example().expect("Simple ring_buffer_example_failed");

    println!("\n{}", "=".repeat(50));

    // Example 2: Sequenced ring buffer
    sequenced_ring_buffer_example().expect("Sequenced ring_buffer_example_failed");

    println!("\n{}", "=".repeat(50));

    // Example 3: Multi-threaded producer-consumer
    multi_threaded_example().expect("Multi-threaded example failed");
}

fn simple_ring_buffer_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n1. Simple Ring Buffer Example");
    
    let buffer: RingBuffer<i32> = RingBuffer::new(8)?;
    let producer = buffer.producer();
    let consumer = buffer.consumer();

    println!("Created ring buffer with capacity: {}", buffer.capacity());

    // Producer: Add some items
    println!("\nProducing items...");
    for i in 1..=5 {
        producer.try_push(i * 10)?;
        println!("  Produced: {}", i * 10);
    }

    println!("Buffer length after production: {}", buffer.len());
    println!("Available space: {}", producer.available_space());

    // Consumer: Read items
    println!("\nConsuming items...");
    while !buffer.is_empty() {
        match consumer.try_pop() {
            Ok(value) => println!("  Consumed: {}", value),
            Err(RenoirError::BufferEmpty { .. }) => break,
            Err(e) => return Err(Box::new(e)),
        }
    }

    println!("Buffer length after consumption: {}", buffer.len());

    Ok(())
}

fn sequenced_ring_buffer_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n2. Sequenced Ring Buffer Example");
    
    let buffer: SequencedRingBuffer<String> = SequencedRingBuffer::new(4)?;
    println!("Created sequenced ring buffer with capacity: {}", buffer.capacity());

    // Write some messages
    println!("\nWriting messages...");
    for i in 1..=3 {
        let guard = buffer.try_claim()?;
        let message = format!("Message {}", i);
        println!("  Writing: {}", message);
        guard.write(message);
    }

    // Read messages
    println!("\nReading messages...");
    for _ in 0..3 {
        match buffer.try_read() {
            Ok(message) => println!("  Read: {}", message),
            Err(e) => {
                println!("  Error reading: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}

fn multi_threaded_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n3. Multi-threaded Producer-Consumer Example");
    
    let buffer = Arc::new(RingBuffer::<u64>::new(1024)?);
    let running = Arc::new(AtomicBool::new(true));
    let start_time = Instant::now();
    
    // Create producer thread
    let producer_buffer = Arc::clone(&buffer);
    let producer_running = Arc::clone(&running);
    let producer_handle = thread::spawn(move || {
        let producer = producer_buffer.producer();
        let mut counter = 0u64;
        let mut produced = 0u64;
        
        while producer_running.load(Ordering::Relaxed) {
            match producer.try_push(counter) {
                Ok(()) => {
                    counter += 1;
                    produced += 1;
                    
                    // Produce at ~100Hz
                    thread::sleep(Duration::from_millis(10));
                }
                Err(RenoirError::BufferFull { .. }) => {
                    thread::sleep(Duration::from_micros(100));
                }
                Err(e) => {
                    eprintln!("Producer error: {:?}", e);
                    break;
                }
            }
        }
        
        println!("Producer thread finished. Produced: {} items", produced);
        produced
    });

    // Create consumer thread
    let consumer_buffer = Arc::clone(&buffer);
    let consumer_running = Arc::clone(&running);
    let consumer_handle = thread::spawn(move || {
        let consumer = consumer_buffer.consumer();
        let mut consumed = 0u64;
        let mut last_value = 0u64;
        
        while consumer_running.load(Ordering::Relaxed) {
            match consumer.try_pop() {
                Ok(value) => {
                    consumed += 1;
                    
                    // Check for lost items (simple sequence check)
                    if value != last_value + 1 && consumed > 1 {
                        println!("  Warning: Possible lost item. Expected: {}, Got: {}", 
                                last_value + 1, value);
                    }
                    last_value = value;
                    
                    if consumed % 100 == 0 {
                        println!("  Consumed {} items, latest value: {}", consumed, value);
                    }
                    
                    // Consume at variable rate
                    thread::sleep(Duration::from_millis(5));
                }
                Err(RenoirError::BufferEmpty { .. }) => {
                    thread::sleep(Duration::from_micros(500));
                }
                Err(e) => {
                    eprintln!("Consumer error: {:?}", e);
                    break;
                }
            }
        }
        
        println!("Consumer thread finished. Consumed: {} items", consumed);
        consumed
    });

    // Let it run for 2 seconds
    thread::sleep(Duration::from_secs(2));
    
    // Signal threads to stop
    running.store(false, Ordering::Relaxed);
    
    // Wait for threads to finish
    let produced = producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    let elapsed = start_time.elapsed();
    
    println!("\nMulti-threaded example results:");
    println!("  Runtime: {:.2}s", elapsed.as_secs_f64());
    println!("  Produced: {} items ({:.1}/sec)", produced, produced as f64 / elapsed.as_secs_f64());
    println!("  Consumed: {} items ({:.1}/sec)", consumed, consumed as f64 / elapsed.as_secs_f64());
    println!("  Buffer final length: {}", buffer.len());
    
    if consumed == produced {
        println!("  ✓ All produced items were consumed!");
    } else {
        println!("  ⚠ Items produced != consumed (difference: {})", 
                produced.abs_diff(consumed));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renoir::ringbuf::RingBuffer;

    #[test]
    fn test_ring_buffer_basic_operations() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        // Test empty buffer
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert!(consumer.try_pop().is_err());

        // Test production
        producer.try_push(1).unwrap();
        producer.try_push(2).unwrap();
        
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 2);

        // Test consumption
        assert_eq!(consumer.try_pop().unwrap(), 1);
        assert_eq!(consumer.try_pop().unwrap(), 2);
        
        assert!(buffer.is_empty());
        assert!(consumer.try_pop().is_err());
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
    fn test_sequenced_ring_buffer() {
        let buffer: SequencedRingBuffer<i32> = SequencedRingBuffer::new(4).unwrap();
        
        let guard = buffer.try_claim().unwrap();
        guard.write(42);
        
        assert_eq!(buffer.try_read().unwrap(), 42);
        assert!(buffer.try_read().is_err());
    }
}