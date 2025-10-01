/**
 * @file region_manager_example.cpp
 * @brief Comprehensive example demonstrating Renoir shared memory region management
 * 
 * This example shows how to:
 * - Create and manage a shared memory manager
 * - Create, open, and manage shared memory regions
 * - Handle region metadata and statistics
 * - Proper cleanup and error handling
 */

#include <iostream>
#include <string>
#include <vector>
#include <chrono>
#include <thread>
#include <iomanip>
#include <cstring>

extern "C" {
#include "renoir.h"
}

class RenoirRegionManagerExample {
private:
    RenoirManagerHandle manager = nullptr;
    std::vector<RenoirRegionHandle> regions;

    void print_header(const std::string& title) {
        std::cout << "\n" << std::string(50, '=') << std::endl;
        std::cout << "  " << title << std::endl;
        std::cout << std::string(50, '=') << std::endl;
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

    void print_region_stats(const RenoirRegionStats& stats) {
        std::cout << "  Total Size: " << stats.total_size << " bytes" << std::endl;
        std::cout << "  Used Size: " << stats.used_size << " bytes" << std::endl;
        std::cout << "  Free Size: " << stats.free_size << " bytes" << std::endl;
        std::cout << "  Fragmentation: " << std::fixed << std::setprecision(1) 
                  << (stats.fragmentation_ratio * 100.0) << "%" << std::endl;
        std::cout << "  Topic Count: " << stats.topic_count << std::endl;
        std::cout << "  Allocations: " << stats.allocation_count << std::endl;
    }

    const char* backing_type_to_string(uint32_t backing_type) {
        switch (backing_type) {
            case 0: return "FileBacked";
            case 1: return "MemFd";
            default: return "Unknown";
        }
    }

public:
    void run_example() {
        print_header("Renoir Region Manager Example");
        
        try {
            create_manager();
            create_regions();
            demonstrate_region_operations();
            demonstrate_statistics();
            cleanup();
            
            std::cout << "\n✓ Region manager example completed successfully!" << std::endl;
            
        } catch (const std::exception& e) {
            std::cerr << "Exception: " << e.what() << std::endl;
            cleanup();
            exit(1);
        }
    }

private:
    void create_manager() {
        print_section("Creating Manager");

        // Create manager (no config needed)
        manager = renoir_manager_create();
        if (!manager) {
            std::cerr << "Failed to create manager" << std::endl;
            exit(1);
        }
        std::cout << "✓ Manager created successfully" << std::endl;
    }

    void create_regions() {
        print_section("Creating Regions");

        struct RegionSpec {
            std::string name;
            size_t size;
            uint32_t backing_type;
            std::string description;
        };

        std::vector<RegionSpec> region_specs = {
            {"small_region", 512 * 1024, 1, "Small MemFd region (512KB)"},
            {"medium_region", 2 * 1024 * 1024, 1, "Medium MemFd region (2MB)"},
            {"large_region", 8 * 1024 * 1024, 0, "Large FileBacked region (8MB)"},
        };

        for (const auto& spec : region_specs) {
            std::cout << "\nCreating region: " << spec.name << std::endl;
            std::cout << "  " << spec.description << std::endl;

            RenoirRegionConfig config = {
                spec.name.c_str(),    // name
                spec.size,            // size
                spec.backing_type,    // backing_type
                nullptr,              // file_path (auto-generated)
                true,                 // create
                0644                  // permissions
            };

            RenoirRegionHandle region;
            RenoirErrorCode error = renoir_region_create(manager, &config, &region);
            check_error(error, "creating region " + spec.name);

            regions.push_back(region);

            std::cout << "✓ Region created:" << std::endl;
            std::cout << "  Size: " << spec.size << " bytes" << std::endl;
            std::cout << "  Backing: " << backing_type_to_string(spec.backing_type) << std::endl;
        }
    }

    void demonstrate_region_operations() {
        print_section("Demonstrating Region Operations");

        for (size_t i = 0; i < regions.size(); ++i) {
            std::cout << "\n--- Region " << (i + 1) << " Operations ---" << std::endl;

            // Get region statistics
            RenoirRegionStats stats;
            RenoirErrorCode error = renoir_region_stats(regions[i], &stats);
            check_error(error, "getting region stats");

            std::cout << "Region Statistics:" << std::endl;
            print_region_stats(stats);

            // Get region size (correct API usage - returns size directly)
            size_t size = renoir_region_size(regions[i]);
            std::cout << "Reported Size: " << size << " bytes" << std::endl;

            // Get region name
            char* region_name = renoir_region_name(regions[i]);
            if (region_name) {
                std::cout << "Region Name: " << region_name << std::endl;
            }
        }
    }

    void demonstrate_statistics() {
        print_section("Demonstrating Statistics");

        // Get total memory usage across all regions
        size_t total_memory = 0;
        RenoirErrorCode error = renoir_manager_total_memory_usage(manager, &total_memory);
        check_error(error, "getting total memory usage");

        std::cout << "Manager Statistics:" << std::endl;
        std::cout << "  Total Memory Usage: " << total_memory << " bytes" << std::endl;

        // Get region count
        size_t region_count = 0;
        error = renoir_manager_region_count(manager, &region_count);
        check_error(error, "getting region count");
        std::cout << "  Region Count: " << region_count << std::endl;

        // List all region names
        char** region_names = nullptr;
        size_t name_count = 0;
        error = renoir_manager_list_regions(manager, &region_names, &name_count);
        check_error(error, "listing regions");

        std::cout << "\nRegistered Regions:" << std::endl;
        if (region_names && name_count > 0) {
            for (size_t i = 0; i < name_count; ++i) {
                std::cout << "  " << (i + 1) << ". " << region_names[i] << std::endl;
            }

            // Free the region names list (correct API - void return)
            renoir_free_region_names(region_names, name_count);
        } else {
            std::cout << "  No regions found" << std::endl;
        }

        // Individual region statistics
        std::cout << "\nDetailed Region Statistics:" << std::endl;
        for (size_t i = 0; i < regions.size(); ++i) {
            RenoirRegionStats stats;
            error = renoir_region_stats(regions[i], &stats);
            if (error == Success) {
                std::cout << "\nRegion " << (i + 1) << ":" << std::endl;
                print_region_stats(stats);

                // Calculate utilization
                double utilization = 0.0;
                if (stats.total_size > 0) {
                    utilization = (double)stats.used_size / stats.total_size;
                }
                std::cout << "  Utilization: " << std::fixed << std::setprecision(1) 
                          << (utilization * 100.0) << "%" << std::endl;
            }
        }
    }

    void cleanup() {
        print_section("Cleanup");

        // Close all regions
        for (auto region : regions) {
            RenoirErrorCode error = renoir_region_close(region);
            if (error == Success) {
                std::cout << "✓ Region closed" << std::endl;
            }
        }
        regions.clear();

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
    std::cout << "Starting Renoir Region Manager Example..." << std::endl;
    
    RenoirRegionManagerExample example;
    example.run_example();
    
    return 0;
}