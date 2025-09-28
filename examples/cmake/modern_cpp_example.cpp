/**
 * @file modern_cpp_example.cpp
 * @brief Modern C++ example using Renoir with RAII and smart pointers
 */

#include "renoir.hpp"
#include <iostream>
#include <vector>
#include <memory>
#include <thread>
#include <chrono>
#include <random>
#include <algorithm>
#include <atomic>

using namespace renoir;
using namespace std::chrono_literals;

class HighPerformanceDataProcessor {
private:
    std::unique_ptr<SharedMemoryManager> manager_;
    std::unique_ptr<SharedMemoryRegion> region_;
    std::unique_ptr<BufferPool> pool_;

public:
    HighPerformanceDataProcessor() {
        // Initialize Renoir
        renoir::initialize();
        
        // Create manager and region with RAII
        manager_ = std::make_unique<SharedMemoryManager>();
        
        RegionConfig region_config{
            .name = "modern_cpp_region",
            .size = 64 * 1024 * 1024, // 64MB
            .use_memfd = true,         // Use memfd for better performance
            .create = true,
            .permissions = 0644
        };
        
        region_ = std::make_unique<SharedMemoryRegion>(
            manager_->createRegion(region_config)
        );
        
        BufferPoolConfig pool_config{
            .name = "high_perf_pool",
            .buffer_size = 8192,      // 8KB buffers
            .initial_count = 100,
            .max_count = 1000,
            .alignment = 64,          // Cache line alignment
            .pre_allocate = true,
            .allocation_timeout = 50ms
        };
        
        pool_ = std::make_unique<BufferPool>(
            region_->createBufferPool(pool_config)
        );
        
        std::cout << "High-performance data processor initialized\n";
        printStats();
    }
    
    void processDataBatch(const std::vector<std::string>& data) {
        std::vector<Buffer> buffers;
        buffers.reserve(data.size());
        
        auto start = std::chrono::high_resolution_clock::now();
        
        // Allocate buffers and write data
        for (size_t i = 0; i < data.size(); ++i) {
            try {
                auto buffer = pool_->getBuffer();
                buffer.writeString(data[i]);
                buffer.setSequence(i);
                buffers.push_back(std::move(buffer));
            } catch (const Exception& e) {
                std::cerr << "Buffer allocation failed: " << e.what() << std::endl;
                break;
            }
        }
        
        auto alloc_time = std::chrono::high_resolution_clock::now();
        
        // Process buffers (simulate work)
        for (auto& buffer : buffers) {
            processBuffer(buffer);
        }
        
        auto process_time = std::chrono::high_resolution_clock::now();
        
        // Buffers are automatically returned when they go out of scope
        
        auto end_time = std::chrono::high_resolution_clock::now();
        
        // Print timing statistics
        auto alloc_duration = std::chrono::duration_cast<std::chrono::microseconds>(
            alloc_time - start).count();
        auto process_duration = std::chrono::duration_cast<std::chrono::microseconds>(
            process_time - alloc_time).count();
        auto total_duration = std::chrono::duration_cast<std::chrono::microseconds>(
            end_time - start).count();
        
        std::cout << "Processed " << buffers.size() << " items:\n";
        std::cout << "  Allocation: " << alloc_duration << "μs\n";
        std::cout << "  Processing: " << process_duration << "μs\n";
        std::cout << "  Total: " << total_duration << "μs\n";
        std::cout << "  Rate: " << (data.size() * 1000000.0 / total_duration) << " items/sec\n";
    }
    
    void printStats() const {
        auto stats = pool_->getStats();
        std::cout << "\nBuffer Pool Statistics:\n";
        std::cout << "  Total allocated: " << stats.total_allocated << "\n";
        std::cout << "  Currently in use: " << stats.currently_in_use << "\n";
        std::cout << "  Peak usage: " << stats.peak_usage << "\n";
        std::cout << "  Success rate: " << (stats.success_rate * 100.0) << "%\n";
        std::cout << "  Utilization: " << (stats.utilization * 100.0) << "%\n\n";
    }

private:
    void processBuffer(const Buffer& buffer) {
        // Simulate some processing work
        const auto* data = buffer.data();
        size_t checksum = 0;
        for (size_t i = 0; i < buffer.size(); ++i) {
            checksum += data[i];
        }
        
        // Prevent optimization from removing the work
        volatile size_t result = checksum;
        (void)result;
        
        // Simulate variable processing time
        std::this_thread::sleep_for(std::chrono::microseconds(10 + (checksum % 20)));
    }
};

// Template for type-safe buffer operations
template<typename T>
class TypedBuffer {
private:
    Buffer buffer_;

public:
    explicit TypedBuffer(Buffer&& buffer) : buffer_(std::move(buffer)) {
        if (buffer_.capacity() < sizeof(T)) {
            throw Exception(RENOIR_INVALID_PARAMETER, "Buffer too small for type");
        }
    }
    
    T* get() { return buffer_.template as<T>(); }
    const T* get() const { return buffer_.template as<const T>(); }
    
    void set(const T& value) {
        *get() = value;
        buffer_.resize(sizeof(T));
    }
    
    Buffer& underlying() { return buffer_; }
    const Buffer& underlying() const { return buffer_; }
};

// Example data structure
struct SensorReading {
    uint64_t timestamp;
    float temperature;
    float humidity;
    float pressure;
    uint32_t sensor_id;
};

void demonstrateTypedBuffers() {
    std::cout << "\n=== Typed Buffer Example ===\n";
    
    SharedMemoryManager manager;
    auto region = manager.createRegion({
        .name = "typed_buffer_region",
        .size = 1024 * 1024,
        .create = true
    });
    
    auto pool = region.createBufferPool({
        .name = "sensor_pool",
        .buffer_size = sizeof(SensorReading),
        .initial_count = 50,
        .max_count = 200
    });
    
    // Create typed buffers
    std::vector<TypedBuffer<SensorReading>> sensor_buffers;
    
    for (int i = 0; i < 10; ++i) {
        auto buffer = pool.getBuffer();
        TypedBuffer<SensorReading> typed_buffer(std::move(buffer));
        
        // Set sensor data
        SensorReading reading{
            .timestamp = static_cast<uint64_t>(
                std::chrono::system_clock::now().time_since_epoch().count()),
            .temperature = 20.0f + (i * 2.5f),
            .humidity = 45.0f + (i * 1.2f),
            .pressure = 1013.25f + (i * 0.5f),
            .sensor_id = static_cast<uint32_t>(100 + i)
        };
        
        typed_buffer.set(reading);
        sensor_buffers.push_back(std::move(typed_buffer));
    }
    
    // Process sensor readings
    for (const auto& typed_buffer : sensor_buffers) {
        const auto* reading = typed_buffer.get();
        std::cout << "Sensor " << reading->sensor_id 
                  << ": T=" << reading->temperature << "°C"
                  << ", H=" << reading->humidity << "%"
                  << ", P=" << reading->pressure << "hPa\n";
    }
}

void demonstrateAsynchronousProcessing() {
    std::cout << "\n=== Asynchronous Processing Example ===\n";
    
    SharedMemoryManager manager;
    auto region = manager.createRegion({
        .name = "async_region",
        .size = 16 * 1024 * 1024,
        .create = true
    });
    
    auto pool = region.createBufferPool({
        .name = "async_pool",
        .buffer_size = 1024,
        .initial_count = 20,
        .max_count = 100,
        .allocation_timeout = 100ms
    });
    
    std::atomic<int> completed_tasks{0};
    const int num_tasks = 50;
    
    // Launch producer thread
    std::thread producer([&]() {
        std::random_device rd;
        std::mt19937 gen(rd());
        std::uniform_int_distribution<> size_dist(100, 800);
        
        for (int i = 0; i < num_tasks; ++i) {
            try {
                auto buffer = pool.getBuffer();
                
                // Generate random data
                size_t data_size = size_dist(gen);
                std::string data = "Task " + std::to_string(i) + " data: ";
                data.resize(data_size, 'A' + (i % 26));
                
                buffer.writeString(data);
                buffer.setSequence(i);
                
                // Simulate variable production rate
                std::this_thread::sleep_for(
                    std::chrono::milliseconds(10 + (i % 30)));
                
            } catch (const Exception& e) {
                std::cerr << "Producer error: " << e.what() << std::endl;
            }
        }
        
        std::cout << "Producer completed " << num_tasks << " tasks\n";
    });
    
    // Launch consumer threads
    std::vector<std::thread> consumers;
    const int num_consumers = 3;
    
    for (int c = 0; c < num_consumers; ++c) {
        consumers.emplace_back([&, c]() {
            int local_completed = 0;
            
            while (completed_tasks.load() < num_tasks) {
                try {
                    auto buffer = pool.getBuffer();
                    
                    // Simulate processing
                    std::this_thread::sleep_for(std::chrono::milliseconds(20));
                    
                    local_completed++;
                    completed_tasks++;
                    
                } catch (const Exception& e) {
                    // Buffer allocation might fail if pool is full
                    std::this_thread::sleep_for(std::chrono::milliseconds(1));
                }
            }
            
            std::cout << "Consumer " << c << " completed " << local_completed << " tasks\n";
        });
    }
    
    producer.join();
    for (auto& consumer : consumers) {
        consumer.join();
    }
    
    pool.getStats();
}

int main() {
    try {
        std::cout << "Modern C++ Renoir Example\n";
        std::cout << "========================\n";
        
        auto [major, minor, patch] = renoir::getVersion();
        std::cout << "Renoir version: " << major << "." << minor << "." << patch << "\n";
        std::cout << "Version string: " << renoir::getVersionString() << "\n\n";
        
        // High-performance batch processing
        HighPerformanceDataProcessor processor;
        
        std::vector<std::string> test_data;
        for (int i = 0; i < 100; ++i) {
            test_data.push_back("Data item " + std::to_string(i) + " with some content");
        }
        
        processor.processDataBatch(test_data);
        processor.printStats();
        
        // Typed buffer demonstration
        demonstrateTypedBuffers();
        
        // Asynchronous processing
        demonstrateAsynchronousProcessing();
        
        std::cout << "\nModern C++ example completed successfully!\n";
        
    } catch (const Exception& e) {
        std::cerr << "Renoir error: " << e.what() << " (code: " << e.code() << ")\n";
        return 1;
    } catch (const std::exception& e) {
        std::cerr << "Standard error: " << e.what() << "\n";
        return 1;
    }
    
    return 0;
}