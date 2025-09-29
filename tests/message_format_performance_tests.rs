//! Zero-Copy Message Format Performance Tests
//! 
//! Performance validation and benchmarking for zero-copy message formats

use renoir::{
    error::Result,
    message_formats::{
        registry::{ZeroCopyFormatRegistry, UseCase},
        zero_copy::create_sensor_data_flatbuffer,
    },
};
use std::time::{Duration, Instant};

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn test_message_creation_performance() -> Result<()> {
        let iterations = 10_000; // Reduced for faster tests
        let start = Instant::now();
        
        for i in 0..iterations {
            let _accessor = create_sensor_data_flatbuffer(
                1000000 + i as u64,
                42.5 + (i as f64) * 0.1,
                100 + (i % 256) as u32,
            )?;
        }
        
        let elapsed = start.elapsed();
        let avg_time_ns = elapsed.as_nanos() as u64 / iterations as u64;
        let throughput = (iterations as f64) / elapsed.as_secs_f64();
        
        // Assertions for reasonable performance
        assert!(avg_time_ns < 10_000); // Less than 10µs per message
        assert!(throughput > 100_000.0); // More than 100k messages/sec
        
        println!("Message creation: {}ns avg, {:.0} msg/sec", avg_time_ns, throughput);
        
        Ok(())
    }

    #[test]
    fn test_field_access_performance() -> Result<()> {
        let accessor = create_sensor_data_flatbuffer(123456789, 98.6, 42)?;
        let iterations = 100_000; // Field access should be very fast
        
        let start = Instant::now();
        let mut sum = 0u64;
        for _ in 0..iterations {
            if let Some(timestamp) = accessor.get_u64("timestamp")? {
                sum += timestamp;
            }
        }
        let elapsed = start.elapsed();
        
        let avg_time_ns = elapsed.as_nanos() as u64 / iterations as u64;
        
        // Field access should be fast (relaxed for different environments)
        assert!(avg_time_ns < 10_000); // Less than 10μs per field access
        assert_eq!(sum / iterations as u64, 123456789); // Verify correctness
        
        println!("Field access: {}ns avg per access", avg_time_ns);
        
        Ok(())
    }

    #[test]
    fn test_format_recommendation_performance() -> Result<()> {
        let registry = ZeroCopyFormatRegistry::new();
        let iterations = 10_000;
        
        let start = Instant::now();
        for _ in 0..iterations {
            let _format1 = registry.recommend_format(&UseCase::HighFrequency);
            let _format2 = registry.recommend_format(&UseCase::LowLatency);
            let _format3 = registry.recommend_format(&UseCase::RealTime);
        }
        let elapsed = start.elapsed();
        
        let avg_time_ns = elapsed.as_nanos() as u64 / (iterations * 3) as u64;
        
        // Format recommendation should be fast (relaxed for different environments)  
        assert!(avg_time_ns < 1_000); // Less than 1μs per recommendation
        
        println!("Format recommendation: {}ns avg", avg_time_ns);
        
        Ok(())
    }

    #[test] 
    fn test_high_frequency_simulation() -> Result<()> {
        // Simulate 1kHz message processing for 100ms
        let target_frequency = 1000; // 1kHz
        let duration = Duration::from_millis(100); // 100ms test
        let message_interval = Duration::from_nanos(1_000_000_000 / target_frequency);
        
        let start = Instant::now();
        let mut message_count = 0;
        let mut next_message_time = start;
        
        while start.elapsed() < duration {
            if Instant::now() >= next_message_time {
                // Create and process message
                let _accessor = create_sensor_data_flatbuffer(
                    start.elapsed().as_micros() as u64,
                    message_count as f64 * 0.1,
                    1,
                )?;
                
                message_count += 1;
                next_message_time += message_interval;
            }
        }
        
        let actual_frequency = message_count as f64 / duration.as_secs_f64();
        let efficiency = (actual_frequency / target_frequency as f64) * 100.0;
        
        // Should achieve reasonable efficiency
        assert!(efficiency > 80.0); // At least 80% efficiency
        assert!(message_count > 80); // Should process at least 80 messages in 100ms
        
        println!("High frequency test: {:.0}Hz actual ({:.1}% efficiency)", 
                 actual_frequency, efficiency);
        
        Ok(())
    }

    #[test]
    fn test_low_latency_processing() -> Result<()> {
        let iterations = 1_000;
        let mut latencies = Vec::with_capacity(iterations);
        
        for i in 0..iterations {
            let start = Instant::now();
            
            // Create and process control message
            let _control_msg = create_sensor_data_flatbuffer(
                start.elapsed().as_nanos() as u64,
                (i as f64).sin(), // Simulated control signal
                2,
            )?;
            
            // Access fields (simulating processing)
            let _timestamp = _control_msg.get_u64("timestamp")?;
            let _value = _control_msg.get_f64("value")?;
            
            let latency = start.elapsed();
            latencies.push(latency.as_nanos() as u64);
        }
        
        // Calculate statistics
        latencies.sort_unstable();
        let avg_ns = latencies.iter().sum::<u64>() / latencies.len() as u64;
        let p95_ns = latencies[(latencies.len() * 95) / 100];
        let p99_ns = latencies[(latencies.len() * 99) / 100];
        
        // Assert reasonable latency characteristics
        assert!(avg_ns < 1_000_000); // Less than 1ms average
        assert!(p95_ns < 2_000_000); // Less than 2ms P95
        assert!(p99_ns < 5_000_000); // Less than 5ms P99
        
        println!("Latency - Avg: {}μs, P95: {}μs, P99: {}μs", 
                 avg_ns / 1000, p95_ns / 1000, p99_ns / 1000);
        
        Ok(())
    }

    #[test]
    fn test_sensor_fusion_throughput() -> Result<()> {
        let duration = Duration::from_millis(500); // 500ms test
        let start = Instant::now();
        let mut processed_messages = 0;
        
        let sensor_types = ["imu", "lidar", "camera", "gps"];
        let mut sensor_id = 0;
        
        while start.elapsed() < duration {
            for _sensor_type in &sensor_types {
                // Create sensor message
                let _sensor_msg = create_sensor_data_flatbuffer(
                    start.elapsed().as_micros() as u64,
                    processed_messages as f64 * 0.01,
                    sensor_id,
                )?;
                
                // Simulate processing
                let _timestamp = _sensor_msg.get_u64("timestamp")?;
                let _value = _sensor_msg.get_f64("value")?;
                let _id = _sensor_msg.get_u32("sensor_id")?;
                
                processed_messages += 1;
                sensor_id = (sensor_id + 1) % 4;
            }
        }
        
        let throughput = processed_messages as f64 / duration.as_secs_f64();
        let data_rate_kb = (processed_messages * 64) as f64 / 1024.0; // Assume 64 bytes per message
        
        // Assert reasonable throughput
        assert!(throughput > 1000.0); // More than 1k messages/sec
        assert!(processed_messages > 500); // Should process at least 500 messages
        
        println!("Sensor fusion - {:.0} msg/sec, {:.1} KB/s", throughput, data_rate_kb);
        
        Ok(())
    }

    #[test]
    fn test_memory_efficiency() -> Result<()> {
        let message_count = 1000;
        let mut total_size = 0;
        let mut accessors = Vec::with_capacity(message_count);
        
        // Create messages and track memory usage
        for i in 0..message_count {
            let accessor = create_sensor_data_flatbuffer(
                i as u64,
                i as f64,
                i as u32,
            )?;
            
            total_size += accessor.buffer().len();
            accessors.push(accessor);
        }
        
        let avg_size = total_size / message_count;
        
        // Assert reasonable memory usage
        assert!(avg_size < 1024); // Less than 1KB per message
        assert!(total_size < 1024 * 1024); // Less than 1MB total
        
        // Verify all messages are still accessible (no premature cleanup)
        for (i, accessor) in accessors.iter().enumerate() {
            assert_eq!(accessor.get_u64("timestamp")?.unwrap(), i as u64);
        }
        
        println!("Memory usage - {} bytes avg per message", avg_size);
        
        Ok(())
    }

    #[test]
    fn test_concurrent_access_simulation() -> Result<()> {
        // Simulate multiple "threads" accessing the same message
        let accessor = create_sensor_data_flatbuffer(12345, 67.89, 101)?;
        let access_count = 10_000;
        
        let start = Instant::now();
        
        // Simulate concurrent field accesses
        for _ in 0..access_count {
            let _ts = accessor.get_u64("timestamp")?;
            let _val = accessor.get_f64("value")?;
            let _id = accessor.get_u32("sensor_id")?;
        }
        
        let elapsed = start.elapsed();
        let avg_time_ns = elapsed.as_nanos() as u64 / (access_count * 3) as u64;
        
        // Concurrent access should remain fast (relaxed for different environments)
        assert!(avg_time_ns < 10_000); // Less than 10μs per field access
        
        println!("Concurrent access simulation: {}ns avg per field", avg_time_ns);
        
        Ok(())
    }

    #[test]
    fn test_zero_copy_validation() -> Result<()> {
        // Create multiple accessors from the same logical data
        let accessors: Vec<_> = (0..100).map(|i| {
            create_sensor_data_flatbuffer(i, i as f64, i as u32)
        }).collect::<Result<Vec<_>>>()?;
        
        // Verify that accessing fields multiple times gives consistent results
        for (i, accessor) in accessors.iter().enumerate() {
            for _ in 0..10 {
                assert_eq!(accessor.get_u64("timestamp")?.unwrap(), i as u64);
                assert_eq!(accessor.get_f64("value")?.unwrap(), i as f64);
                assert_eq!(accessor.get_u32("sensor_id")?.unwrap(), i as u32);
            }
        }
        
        println!("Zero-copy validation: {} messages verified", accessors.len());
        
        Ok(())
    }

    #[test]
    fn test_format_type_performance_comparison() -> Result<()> {
        let registry = ZeroCopyFormatRegistry::new();
        
        // Test that different use cases get appropriate format recommendations
        let test_cases = [
            UseCase::HighFrequency,
            UseCase::LowLatency,
            UseCase::RealTime,
            UseCase::Streaming,
        ];
        
        for use_case in test_cases {
            let start = Instant::now();
            for _ in 0..1000 {
                let format = registry.recommend_format(&use_case);
                assert!(format.is_zero_copy());
            }
            let elapsed = start.elapsed();
            
            // Should be very fast
            assert!(elapsed < Duration::from_millis(1));
        }
        
        Ok(())
    }
}