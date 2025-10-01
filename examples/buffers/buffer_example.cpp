/**
 * @file buffer_example.cpp  
 * @brief Comprehensive example demonstrating Renoir buffer pool management
 * 
 * This example shows how to:
 * - Create and configure buffer pools
 * - Allocate and return buffers
 * - Monitor pool statistics and health
 * - Handle multi-threaded buffer operations
 * - Proper cleanup and error handling
 */

#include <iostream>
#include <string>
#include <vector>
#include <thread>
#include <chrono>
#include <iomanip>
#include <atomic>

extern "C" {
#include "renoir.h"
}

class RenoirBufferExample {
private:
    RenoirManagerHandle manager = nullptr;
    RenoirRegionHandle backing_region = nullptr;
    RenoirBufferPoolHandle buffer_pool = nullptr;

    void print_header(const std::string& title) {
        std::cout << "\n" << std::string(60, '=') << std::endl;
        std::cout << "  " << title << std::endl;
        std::cout << std::string(60, '=') << std::endl;
    }

    void print_section(const std::string& section) {
        std::cout << "\n>>> " << section << std::endl;
    }

    void check_error(RenoirErrorCode error, const std::string& operation) {
        if (error != Success) {
            std::cerr << "Error in " << operation << ": " << static_cast<int>(error) << std::endl;
            cleanup();
            exit(1);
        }
    }

    void print_pool_stats(const RenoirBufferPoolStats& stats) {
        std::cout << "  Total Allocated: " << stats.total_allocated << std::endl;
        std::cout << "  Currently In Use: " << stats.currently_in_use << std::endl;
        std::cout << "  Available: " << (stats.total_allocated - stats.currently_in_use) << std::endl;
        std::cout << "  Peak Usage: " << stats.peak_usage << std::endl;
        std::cout << "  Success Rate: " << std::fixed << std::setprecision(1) 
                  << (stats.success_rate * 100.0) << "%" << std::endl;
        std::cout << "  Utilization: " << std::fixed << std::setprecision(1) 
                  << (stats.utilization * 100.0) << "%" << std::endl;
        std::cout << "  Total Allocations: " << stats.total_allocations << std::endl;
        std::cout << "  Total Deallocations: " << stats.total_deallocations << std::endl;
        std::cout << "  Allocation Failures: " << stats.allocation_failures << std::endl;
        std::cout << "  Health Status: " << (stats.is_healthy ? "Healthy ✓" : "Unhealthy ⚠") << std::endl;
    }

public:
    void run_example() {
        print_header("Renoir Buffer Pool Example");
        
        try {
            setup_infrastructure();
            create_buffer_pool();
            demonstrate_buffer_operations();
            demonstrate_statistics();
            stress_test_pool();
            cleanup();
            
            std::cout << "\n✓ Buffer pool example completed successfully!" << std::endl;
            
        } catch (const std::exception& e) {
            std::cerr << "Exception: " << e.what() << std::endl;
            cleanup();
            exit(1);
        }
    }

private:
    void setup_infrastructure() {
        print_section("Setting up Infrastructure");

        // Create manager
        manager = renoir_manager_create();
        if (!manager) {
            std::cerr << "Failed to create manager" << std::endl;
            exit(1);
        }
        std::cout << "✓ Manager created" << std::endl;

        // Create backing region
        RenoirRegionConfig region_config = {
            "buffer_pool_region",  // name
            2 * 1024 * 1024,       // size (2MB)
            1,                     // backing_type (MemFd)
            nullptr,               // file_path
            true,                  // create
            0644                   // permissions
        };

        RenoirErrorCode error = renoir_region_create(manager, &region_config, &backing_region);
        check_error(error, "creating backing region");
        std::cout << "✓ Backing region created (2MB MemFd)" << std::endl;
    }

    void create_buffer_pool() {
        print_section("Creating Buffer Pool");

        RenoirBufferPoolConfig pool_config = {
            "example_pool",        // name
            4096,                  // buffer_size (4KB buffers)
            10,                    // initial_count (start with 10 buffers)
            50,                    // max_count (allow up to 50 buffers)
            64,                    // alignment (64-byte alignment)
            true,                  // pre_allocate (allocate initial buffers immediately)
            1000                   // allocation_timeout_ms (1 second timeout)
        };

        RenoirErrorCode error = renoir_buffer_pool_create(manager, "buffer_pool_region", &pool_config, &buffer_pool);
        check_error(error, "creating buffer pool");

        std::cout << "✓ Buffer pool created with configuration:" << std::endl;
        std::cout << "  Buffer Size: " << pool_config.buffer_size << " bytes" << std::endl;
        std::cout << "  Initial Count: " << pool_config.initial_count << std::endl;
        std::cout << "  Max Count: " << pool_config.max_count << std::endl;
        std::cout << "  Alignment: " << pool_config.alignment << " bytes" << std::endl;

        // Get initial pool statistics
        RenoirBufferPoolStats stats;
        error = renoir_buffer_pool_stats(buffer_pool, &stats);
        check_error(error, "getting initial pool stats");

        std::cout << "\nInitial Pool Statistics:" << std::endl;
        print_pool_stats(stats);
    }

    void demonstrate_buffer_operations() {
        print_section("Demonstrating Buffer Operations");

        std::cout << "\nAllocating and using buffers..." << std::endl;

        std::vector<RenoirBufferHandle> allocated_buffers;

        // Allocate several buffers
        for (int i = 0; i < 5; ++i) {
            RenoirBufferHandle buffer;
            RenoirErrorCode error = renoir_buffer_get(buffer_pool, &buffer);
            check_error(error, "allocating buffer " + std::to_string(i + 1));
            
            allocated_buffers.push_back(buffer);
            std::cout << "✓ Allocated buffer " << (i + 1) << std::endl;

            // Get buffer info
            RenoirBufferInfo info;
            error = renoir_buffer_info(buffer, &info);
            if (error == Success) {
                std::cout << "  Buffer size: " << info.size << " bytes" << std::endl;
                std::cout << "  Buffer capacity: " << info.capacity << " bytes" << std::endl;
                std::cout << "  Buffer alignment: " << info.alignment << " bytes" << std::endl;
                if (info.pool_name) {
                    std::cout << "  Pool name: " << info.pool_name << std::endl;
                }
            }
        }

        // Show pool stats after allocation
        RenoirBufferPoolStats stats;
        RenoirErrorCode error = renoir_buffer_pool_stats(buffer_pool, &stats);
        check_error(error, "getting pool stats after allocation");

        std::cout << "\nPool Statistics After Allocation:" << std::endl;
        print_pool_stats(stats);

        // Return buffers
        std::cout << "\nReturning buffers..." << std::endl;
        for (size_t i = 0; i < allocated_buffers.size(); ++i) {
            error = renoir_buffer_return(buffer_pool, allocated_buffers[i]);
            check_error(error, "returning buffer " + std::to_string(i + 1));
            std::cout << "✓ Returned buffer " << (i + 1) << std::endl;
        }
        allocated_buffers.clear();

        // Show final stats
        error = renoir_buffer_pool_stats(buffer_pool, &stats);
        check_error(error, "getting pool stats after return");

        std::cout << "\nPool Statistics After Return:" << std::endl;
        print_pool_stats(stats);
    }

    void demonstrate_statistics() {
        print_section("Demonstrating Statistics Monitoring");

        std::cout << "Monitoring pool statistics over several operations..." << std::endl;

        const int monitoring_cycles = 3;
        
        for (int cycle = 0; cycle < monitoring_cycles; ++cycle) {
            std::cout << "\n--- Cycle " << (cycle + 1) << " ---" << std::endl;

            // Allocate some buffers
            std::vector<RenoirBufferHandle> temp_buffers;
            int alloc_count = 3 + cycle;  // Vary the load
            
            for (int i = 0; i < alloc_count; ++i) {
                RenoirBufferHandle buffer;
                RenoirErrorCode error = renoir_buffer_get(buffer_pool, &buffer);
                if (error == Success) {
                    temp_buffers.push_back(buffer);
                }
            }

            std::cout << "Allocated " << temp_buffers.size() << " buffers" << std::endl;

            // Get statistics
            RenoirBufferPoolStats stats;
            RenoirErrorCode error = renoir_buffer_pool_stats(buffer_pool, &stats);
            if (error == Success) {
                std::cout << "Current Usage: " << stats.currently_in_use 
                          << "/" << stats.total_allocated << std::endl;
                std::cout << "Success Rate: " << std::fixed << std::setprecision(1) 
                          << (stats.success_rate * 100.0) << "%" << std::endl;
            }

            // Brief pause to simulate work
            std::this_thread::sleep_for(std::chrono::milliseconds(100));

            // Return buffers
            for (auto buffer : temp_buffers) {
                renoir_buffer_return(buffer_pool, buffer);
            }

            std::cout << "Returned all buffers" << std::endl;
        }
    }

    void stress_test_pool() {
        print_section("Stress Testing Pool");

        std::cout << "Running multi-threaded stress test..." << std::endl;

        const int num_threads = 4;
        const int operations_per_thread = 100;
        std::atomic<int> successful_operations{0};
        std::atomic<int> failed_operations{0};

        auto worker = [&](int thread_id) {
            for (int i = 0; i < operations_per_thread; ++i) {
                RenoirBufferHandle buffer;
                RenoirErrorCode error = renoir_buffer_get(buffer_pool, &buffer);
                
                if (error == Success) {
                    // Simulate some work
                    std::this_thread::sleep_for(std::chrono::microseconds(10));
                    
                    // Return buffer
                    error = renoir_buffer_return(buffer_pool, buffer);
                    if (error == Success) {
                        successful_operations++;
                    } else {
                        failed_operations++;
                    }
                } else {
                    failed_operations++;
                }
            }
        };

        auto start_time = std::chrono::high_resolution_clock::now();

        // Launch worker threads
        std::vector<std::thread> threads;
        for (int i = 0; i < num_threads; ++i) {
            threads.emplace_back(worker, i);
        }

        // Wait for all threads to complete
        for (auto& thread : threads) {
            thread.join();
        }

        auto end_time = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::milliseconds>(end_time - start_time);

        std::cout << "\nStress Test Results:" << std::endl;
        std::cout << "  Threads: " << num_threads << std::endl;
        std::cout << "  Operations per thread: " << operations_per_thread << std::endl;
        std::cout << "  Total operations: " << (num_threads * operations_per_thread) << std::endl;
        std::cout << "  Successful operations: " << successful_operations.load() << std::endl;
        std::cout << "  Failed operations: " << failed_operations.load() << std::endl;
        std::cout << "  Duration: " << duration.count() << " ms" << std::endl;
        
        if (successful_operations.load() > 0) {
            double ops_per_second = (double)successful_operations.load() / (duration.count() / 1000.0);
            std::cout << "  Performance: " << std::fixed << std::setprecision(1) 
                      << ops_per_second << " ops/sec" << std::endl;
        }

        // Final pool statistics
        RenoirBufferPoolStats final_stats;
        RenoirErrorCode error = renoir_buffer_pool_stats(buffer_pool, &final_stats);
        if (error == Success) {
            std::cout << "\nFinal Pool Statistics:" << std::endl;
            print_pool_stats(final_stats);
        }
    }

    void cleanup() {
        print_section("Cleanup");

        // Destroy buffer pool
        if (buffer_pool) {
            RenoirErrorCode error = renoir_buffer_pool_destroy(buffer_pool);
            if (error == Success) {
                std::cout << "✓ Buffer pool destroyed" << std::endl;
            }
            buffer_pool = nullptr;
        }

        // Close backing region
        if (backing_region) {
            RenoirErrorCode error = renoir_region_close(backing_region);
            if (error == Success) {
                std::cout << "✓ Backing region closed" << std::endl;
            }
            backing_region = nullptr;
        }

        // Destroy manager
        if (manager) {
            RenoirErrorCode error = renoir_manager_destroy(manager);
            if (error == Success) {
                std::cout << "✓ Manager destroyed" << std::endl;
            }
            manager = nullptr;
        }
    }
};

int main() {
    std::cout << "Starting Renoir Buffer Pool Example..." << std::endl;
    
    RenoirBufferExample example;
    example.run_example();
    
    return 0;
}