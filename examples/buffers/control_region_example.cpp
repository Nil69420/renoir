/**
 * @file control_region_example.cpp
 * @brief Comprehensive example demonstrating Renoir control region and metadata operations
 * 
 * This example shows how to:
 * - Create and manage control regions
 * - Register and manage buffer pool metadata
 * - Monitor control region statistics
 * - Handle region registry operations
 * - Cleanup stale entries
 */

#include <iostream>
#include <string>
#include <vector>
#include <chrono>
#include <thread>
#include <iomanip>

extern "C" {
#include "renoir.h"
}

class RenoirControlRegionExample {
private:
    RenoirManagerHandle manager = nullptr;
    RenoirRegionHandle backing_region = nullptr;
    RenoirControlRegionHandle control_region = nullptr;
    std::vector<RenoirBufferPoolHandle> buffer_pools;

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

    void print_control_stats(const RenoirControlStats& stats) {
        std::cout << "  Sequence Number: " << stats.sequence_number << std::endl;
        std::cout << "  Total Regions: " << stats.total_regions << std::endl;
        std::cout << "  Active Regions: " << stats.active_regions << std::endl;
        std::cout << "  Stale Regions: " << stats.stale_regions << std::endl;
    }

    const char* backing_type_to_string(uint32_t backing_type) {
        switch (backing_type) {
            case 0: return "FileBacked";
            case 1: return "MemFd";
            default: return "Unknown";
        }
    }

    void print_region_entry(const RenoirRegionRegistryEntry& entry, size_t index) {
        std::cout << "  Region " << (index + 1) << ":" << std::endl;
        std::cout << "    Name: " << entry.name << std::endl;
        std::cout << "    Size: " << entry.size << " bytes" << std::endl;
        std::cout << "    Backing Type: " << backing_type_to_string(entry.backing_type) << std::endl;
        std::cout << "    Version: " << entry.version << std::endl;
        std::cout << "    Ref Count: " << entry.ref_count << std::endl;
        std::cout << "    Creator PID: " << entry.creator_pid << std::endl;
    }

public:
    void run_example() {
        print_header("Renoir Control Region & Metadata Example");
        
        try {
            setup_infrastructure();
            create_control_region();
            create_sample_buffer_pools();
            register_buffer_pools();
            demonstrate_registry_operations();
            demonstrate_statistics_monitoring();
            demonstrate_cleanup_operations();
            cleanup();
            
            std::cout << "\n✓ Control region example completed successfully!" << std::endl;
            
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

        // Create backing region for buffer pools
        RenoirRegionConfig region_config = {
            "backing_region",      // name
            4 * 1024 * 1024,       // size (4MB)
            1,                     // backing_type (MemFd)
            nullptr,               // file_path
            true,                  // create
            0644                   // permissions
        };

        RenoirErrorCode error = renoir_region_create(manager, &region_config, &backing_region);
        check_error(error, "creating backing region");
        std::cout << "✓ Backing region created (4MB MemFd)" << std::endl;
    }

    void create_control_region() {
        print_section("Creating Control Region");

        RenoirRegionConfig control_config = {
            "control_region",      // name
            512 * 1024,            // size (512KB)
            0,                     // backing_type (FileBacked for persistence)
            nullptr,               // file_path
            true,                  // create
            0644                   // permissions
        };

        RenoirErrorCode error = renoir_control_region_create(&control_config, &control_region);
        check_error(error, "creating control region");

        std::cout << "✓ Control region created:" << std::endl;
        std::cout << "  Name: " << control_config.name << std::endl;
        std::cout << "  Size: " << control_config.size << " bytes" << std::endl;
        std::cout << "  Backing: " << backing_type_to_string(control_config.backing_type) << std::endl;

        // Get initial statistics
        RenoirControlStats stats;
        error = renoir_control_region_stats(control_region, &stats);
        check_error(error, "getting initial control stats");

        std::cout << "\nInitial Control Region Statistics:" << std::endl;
        print_control_stats(stats);
    }

    void create_sample_buffer_pools() {
        print_section("Creating Sample Buffer Pools");

        struct PoolSpec {
            std::string name;
            size_t buffer_size;
            size_t initial_count;
            size_t max_count;
        };

        std::vector<PoolSpec> pool_specs = {
            {"small_pool", 1024, 20, 100},
            {"medium_pool", 4096, 10, 50},
            {"large_pool", 16384, 5, 25}
        };

        for (const auto& spec : pool_specs) {
            std::cout << "\nCreating buffer pool: " << spec.name << std::endl;

            RenoirBufferPoolConfig pool_config = {
                spec.name.c_str(),     // name
                spec.buffer_size,      // buffer_size
                spec.initial_count,    // initial_count
                spec.max_count,        // max_count
                64,                    // alignment
                true,                  // pre_allocate
                1000                   // allocation_timeout_ms
            };

            RenoirBufferPoolHandle pool;
            RenoirErrorCode error = renoir_buffer_pool_create(manager, "backing_region", &pool_config, &pool);
            check_error(error, "creating buffer pool " + spec.name);

            buffer_pools.push_back(pool);

            std::cout << "✓ Created pool:" << std::endl;
            std::cout << "  Buffer Size: " << spec.buffer_size << " bytes" << std::endl;
            std::cout << "  Initial Count: " << spec.initial_count << std::endl;
            std::cout << "  Max Count: " << spec.max_count << std::endl;
        }
    }

    void register_buffer_pools() {
        print_section("Registering Buffer Pools with Control Region");

        for (size_t i = 0; i < buffer_pools.size(); ++i) {
            std::cout << "\nRegistering buffer pool " << (i + 1) << "..." << std::endl;

            RenoirErrorCode error = renoir_control_region_register_buffer_pool(control_region, buffer_pools[i]);
            check_error(error, "registering buffer pool");

            std::cout << "✓ Buffer pool registered successfully" << std::endl;
        }

        // Get updated statistics
        RenoirControlStats stats;
        RenoirErrorCode error = renoir_control_region_stats(control_region, &stats);
        check_error(error, "getting updated control stats");

        std::cout << "\nUpdated Control Region Statistics:" << std::endl;
        print_control_stats(stats);
    }

    void demonstrate_registry_operations() {
        print_section("Demonstrating Registry Operations");

        // List all registered regions
        std::cout << "\nListing all registered regions..." << std::endl;

        RenoirRegionRegistryEntry* entries = nullptr;
        size_t count = 0;

        RenoirErrorCode error = renoir_control_region_list_regions(control_region, &entries, &count);
        check_error(error, "listing regions");

        std::cout << "Found " << count << " registered regions:" << std::endl;

        if (entries && count > 0) {
            for (size_t i = 0; i < count; ++i) {
                print_region_entry(entries[i], i);
            }

            // Free the region entries
            error = renoir_free_region_entries(entries, count);
            check_error(error, "freeing region entries");
            std::cout << "\n✓ Region entries list freed" << std::endl;
        } else {
            std::cout << "No regions found in registry." << std::endl;
        }

        // Show some buffer pool statistics to demonstrate the registered pools are working
        std::cout << "\nBuffer Pool Statistics (to verify registration):" << std::endl;
        for (size_t i = 0; i < buffer_pools.size() && i < 2; ++i) {  // Show first 2 pools
            RenoirBufferPoolStats pool_stats;
            error = renoir_buffer_pool_stats(buffer_pools[i], &pool_stats);
            check_error(error, "getting pool stats");

            std::cout << "\nPool " << (i + 1) << " Statistics:" << std::endl;
            std::cout << "  Allocated Buffers: " << pool_stats.total_allocated << std::endl;
            std::cout << "  In Use: " << pool_stats.currently_in_use << std::endl;
            std::cout << "  Success Rate: " << std::fixed << std::setprecision(1) 
                      << (pool_stats.success_rate * 100.0) << "%" << std::endl;
            std::cout << "  Health: " << (pool_stats.is_healthy ? "Healthy ✓" : "Unhealthy ⚠") << std::endl;
        }
    }

    void demonstrate_statistics_monitoring() {
        print_section("Demonstrating Statistics Monitoring");

        // Monitor statistics over time with some activity
        const int monitoring_cycles = 3;
        
        std::cout << "Monitoring control region statistics over " << monitoring_cycles << " cycles..." << std::endl;

        for (int cycle = 0; cycle < monitoring_cycles; ++cycle) {
            std::cout << "\n--- Cycle " << (cycle + 1) << " ---" << std::endl;

            // Get current statistics
            RenoirControlStats stats;
            RenoirErrorCode error = renoir_control_region_stats(control_region, &stats);
            check_error(error, "getting control stats");

            std::cout << "Control Statistics:" << std::endl;
            print_control_stats(stats);

            // Perform some buffer operations to create activity
            if (cycle < monitoring_cycles - 1) {  // Don't do this on last cycle
                std::cout << "\nPerforming buffer operations..." << std::endl;
                
                // Allocate and return some buffers
                std::vector<RenoirBufferHandle> temp_buffers;
                
                for (size_t pool_idx = 0; pool_idx < std::min(buffer_pools.size(), size_t(2)); ++pool_idx) {
                    for (int j = 0; j < 3; ++j) {
                        RenoirBufferHandle buffer;
                        error = renoir_buffer_get(buffer_pools[pool_idx], &buffer);
                        if (error == Success) {
                            temp_buffers.push_back(buffer);
                        }
                    }
                }

                std::cout << "  Allocated " << temp_buffers.size() << " temporary buffers" << std::endl;

                // Brief pause
                std::this_thread::sleep_for(std::chrono::milliseconds(100));

                // Return buffers
                for (size_t pool_idx = 0; pool_idx < std::min(buffer_pools.size(), size_t(2)); ++pool_idx) {
                    while (!temp_buffers.empty()) {
                        RenoirBufferHandle buffer = temp_buffers.back();
                        temp_buffers.pop_back();
                        renoir_buffer_return(buffer_pools[pool_idx], buffer);
                        if (temp_buffers.empty()) break;
                    }
                }

                std::cout << "  Returned all temporary buffers" << std::endl;
            }

            // Pause between cycles
            if (cycle < monitoring_cycles - 1) {
                std::this_thread::sleep_for(std::chrono::milliseconds(200));
            }
        }
    }

    void demonstrate_cleanup_operations() {
        print_section("Demonstrating Cleanup Operations");

        // Test cleanup of stale entries (though we won't have truly stale entries in this example)
        std::cout << "\nTesting stale entry cleanup..." << std::endl;

        size_t cleaned_count;
        uint64_t max_age_seconds = 1;  // Very short age for demonstration

        RenoirErrorCode error = renoir_control_region_cleanup_stale(control_region, max_age_seconds, &cleaned_count);
        check_error(error, "cleaning stale entries");

        std::cout << "✓ Cleanup operation completed:" << std::endl;
        std::cout << "  Entries cleaned: " << cleaned_count << std::endl;
        std::cout << "  Max age threshold: " << max_age_seconds << " seconds" << std::endl;

        // Show final statistics
        RenoirControlStats final_stats;
        error = renoir_control_region_stats(control_region, &final_stats);
        check_error(error, "getting final control stats");

        std::cout << "\nFinal Control Region Statistics:" << std::endl;
        print_control_stats(final_stats);
    }

    void cleanup() {
        print_section("Cleanup");

        // Destroy buffer pools
        for (auto pool : buffer_pools) {
            RenoirErrorCode error = renoir_buffer_pool_destroy(pool);
            if (error == Success) {
                std::cout << "✓ Buffer pool destroyed" << std::endl;
            }
        }
        buffer_pools.clear();

        // Destroy control region
        if (control_region) {
            RenoirErrorCode error = renoir_control_region_destroy(control_region);
            if (error == Success) {
                std::cout << "✓ Control region destroyed" << std::endl;
            }
            control_region = nullptr;
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
    std::cout << "Starting Renoir Control Region & Metadata Example..." << std::endl;
    
    RenoirControlRegionExample example;
    example.run_example();
    
    return 0;
}