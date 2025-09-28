/**
 * @file renoir_example.cpp
 * @brief C++ example using Renoir shared memory library through C API
 */

#include <iostream>
#include <vector>
#include <string>
#include <thread>
#include <chrono>
#include <cstring>
#include "renoir.h"

class RenoirManager {
private:
    RenoirManagerHandle manager_;

public:
    RenoirManager() : manager_(nullptr) {
        manager_ = renoir_manager_create();
        if (!manager_) {
            throw std::runtime_error("Failed to create Renoir manager");
        }
    }

    ~RenoirManager() {
        if (manager_) {
            renoir_manager_destroy(manager_);
        }
    }

    RenoirManagerHandle handle() const { return manager_; }
};

class RenoirRegion {
private:
    RenoirRegionHandle region_;

public:
    RenoirRegion(RenoirManagerHandle manager, const std::string& name, size_t size, 
                 const std::string& file_path = "") : region_(nullptr) {
        RenoirRegionConfig config = {};
        config.name = name.c_str();
        config.size = size;
        config.backing_type = 0; // FileBacked
        config.file_path = file_path.empty() ? nullptr : file_path.c_str();
        config.create = true;
        config.permissions = 0644;

        RenoirErrorCode result = renoir_region_create(manager, &config, &region_);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to create region: " + std::to_string(result));
        }
    }

    ~RenoirRegion() {
        // Region cleanup is handled by manager
    }

    RenoirRegionHandle handle() const { return region_; }
};

class RenoirBufferPool {
private:
    RenoirBufferPoolHandle pool_;

public:
    RenoirBufferPool(RenoirRegionHandle region, const std::string& name, 
                     size_t buffer_size, size_t initial_count, size_t max_count)
        : pool_(nullptr) {
        RenoirBufferPoolConfig config = {};
        config.name = name.c_str();
        config.buffer_size = buffer_size;
        config.initial_count = initial_count;
        config.max_count = max_count;
        config.alignment = 64;
        config.pre_allocate = true;
        config.allocation_timeout_ms = 100;

        RenoirErrorCode result = renoir_buffer_pool_create(region, &config, &pool_);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to create buffer pool: " + std::to_string(result));
        }
    }

    ~RenoirBufferPool() {
        if (pool_) {
            renoir_buffer_pool_destroy(pool_);
        }
    }

    RenoirBufferHandle getBuffer() {
        RenoirBufferHandle buffer;
        RenoirErrorCode result = renoir_buffer_get(pool_, &buffer);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to get buffer: " + std::to_string(result));
        }
        return buffer;
    }

    void returnBuffer(RenoirBufferHandle buffer) {
        RenoirErrorCode result = renoir_buffer_return(pool_, buffer);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to return buffer: " + std::to_string(result));
        }
    }

    RenoirBufferPoolStats getStats() {
        RenoirBufferPoolStats stats;
        RenoirErrorCode result = renoir_buffer_pool_stats(pool_, &stats);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to get pool stats: " + std::to_string(result));
        }
        return stats;
    }

    RenoirBufferPoolHandle handle() const { return pool_; }
};

class RenoirBuffer {
private:
    RenoirBufferHandle buffer_;
    RenoirBufferPool* pool_;
    RenoirBufferInfo info_;

public:
    RenoirBuffer(RenoirBufferPool* pool) : buffer_(nullptr), pool_(pool) {
        buffer_ = pool_->getBuffer();
        
        RenoirErrorCode result = renoir_buffer_info(buffer_, &info_);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to get buffer info: " + std::to_string(result));
        }
    }

    ~RenoirBuffer() {
        if (buffer_ && pool_) {
            pool_->returnBuffer(buffer_);
        }
    }

    // Move constructor
    RenoirBuffer(RenoirBuffer&& other) noexcept 
        : buffer_(other.buffer_), pool_(other.pool_), info_(other.info_) {
        other.buffer_ = nullptr;
        other.pool_ = nullptr;
    }

    // Move assignment
    RenoirBuffer& operator=(RenoirBuffer&& other) noexcept {
        if (this != &other) {
            if (buffer_ && pool_) {
                pool_->returnBuffer(buffer_);
            }
            buffer_ = other.buffer_;
            pool_ = other.pool_;
            info_ = other.info_;
            other.buffer_ = nullptr;
            other.pool_ = nullptr;
        }
        return *this;
    }

    // Disable copy
    RenoirBuffer(const RenoirBuffer&) = delete;
    RenoirBuffer& operator=(const RenoirBuffer&) = delete;

    uint8_t* data() { return info_.data; }
    const uint8_t* data() const { return info_.data; }
    size_t size() const { return info_.size; }
    size_t capacity() const { return info_.capacity; }
    uint64_t sequence() const { return info_.sequence; }

    void resize(size_t new_size) {
        RenoirErrorCode result = renoir_buffer_resize(buffer_, new_size);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to resize buffer: " + std::to_string(result));
        }
        info_.size = new_size;
    }

    void setSequence(uint64_t seq) {
        RenoirErrorCode result = renoir_buffer_set_sequence(buffer_, seq);
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to set sequence: " + std::to_string(result));
        }
        info_.sequence = seq;
    }

    void writeString(const std::string& str) {
        if (str.size() > capacity()) {
            throw std::runtime_error("String too large for buffer");
        }
        std::memcpy(data(), str.c_str(), str.size());
        resize(str.size());
    }

    std::string readString() const {
        return std::string(reinterpret_cast<const char*>(data()), size());
    }
};

void printVersion() {
    std::cout << "Renoir Library Version: " 
              << renoir_version_major() << "."
              << renoir_version_minor() << "."
              << renoir_version_patch() << std::endl;
    
    char* version_str = renoir_version_string();
    if (version_str) {
        std::cout << "Version string: " << version_str << std::endl;
        renoir_free_string(version_str);
    }
}

void basicExample() {
    std::cout << "\n=== Basic Example ===" << std::endl;
    
    try {
        // Initialize Renoir
        RenoirErrorCode result = renoir_init();
        if (result != RENOIR_SUCCESS) {
            throw std::runtime_error("Failed to initialize Renoir");
        }

        // Create manager and region
        RenoirManager manager;
        RenoirRegion region(manager.handle(), "cpp_example", 1024 * 1024, "/tmp/renoir_cpp");

        std::cout << "Created shared memory region successfully" << std::endl;

        // Create buffer pool
        RenoirBufferPool pool(region.handle(), "cpp_pool", 4096, 8, 32);
        std::cout << "Created buffer pool successfully" << std::endl;

        // Test buffer allocation and usage
        std::vector<std::string> messages = {
            "Hello from C++!",
            "Renoir shared memory",
            "Zero-copy communication",
            "High performance IPC"
        };

        std::cout << "\nTesting buffer operations..." << std::endl;
        
        std::vector<RenoirBuffer> buffers;
        
        // Allocate buffers and write data
        for (size_t i = 0; i < messages.size(); ++i) {
            buffers.emplace_back(&pool);
            auto& buffer = buffers.back();
            
            buffer.writeString(messages[i]);
            buffer.setSequence(i);
            
            std::cout << "Buffer " << i << ": '" << buffer.readString() 
                      << "' (seq: " << buffer.sequence() << ")" << std::endl;
        }

        // Get pool statistics
        auto stats = pool.getStats();
        std::cout << "\nBuffer Pool Statistics:" << std::endl;
        std::cout << "  In use: " << stats.currently_in_use << std::endl;
        std::cout << "  Total allocated: " << stats.total_allocated << std::endl;
        std::cout << "  Success rate: " << (stats.success_rate * 100.0) << "%" << std::endl;
        std::cout << "  Utilization: " << (stats.utilization * 100.0) << "%" << std::endl;

        // Buffers will be automatically returned when they go out of scope
        
    } catch (const std::exception& e) {
        std::cerr << "Error in basic example: " << e.what() << std::endl;
    }
}

void performanceTest() {
    std::cout << "\n=== Performance Test ===" << std::endl;
    
    try {
        RenoirManager manager;
        RenoirRegion region(manager.handle(), "perf_test", 64 * 1024 * 1024, "/tmp/renoir_perf"); // 64MB
        RenoirBufferPool pool(region.handle(), "perf_pool", 1024, 100, 1000);

        const size_t num_iterations = 10000;
        const std::string test_data = "Performance test data for Renoir shared memory system";

        auto start = std::chrono::high_resolution_clock::now();

        for (size_t i = 0; i < num_iterations; ++i) {
            RenoirBuffer buffer(&pool);
            buffer.writeString(test_data);
            buffer.setSequence(i);
            
            // Read it back to verify
            std::string read_data = buffer.readString();
            if (read_data != test_data) {
                throw std::runtime_error("Data verification failed");
            }
        }

        auto end = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::microseconds>(end - start);
        
        double ops_per_second = (double)num_iterations / (duration.count() / 1000000.0);
        double avg_latency = (double)duration.count() / num_iterations;

        std::cout << "Performance Results:" << std::endl;
        std::cout << "  Iterations: " << num_iterations << std::endl;
        std::cout << "  Total time: " << duration.count() << " μs" << std::endl;
        std::cout << "  Operations/sec: " << (int)ops_per_second << std::endl;
        std::cout << "  Average latency: " << avg_latency << " μs" << std::endl;

        auto final_stats = pool.getStats();
        std::cout << "  Final success rate: " << (final_stats.success_rate * 100.0) << "%" << std::endl;

    } catch (const std::exception& e) {
        std::cerr << "Error in performance test: " << e.what() << std::endl;
    }
}

int main() {
    std::cout << "Renoir C++ Example" << std::endl;
    std::cout << "==================" << std::endl;
    
    printVersion();
    
    basicExample();
    performanceTest();
    
    std::cout << "\nExample completed!" << std::endl;
    return 0;
}