/**
 * @file ring_buffer_example.cpp
 * @brief Comprehensive example demonstrating Renoir ring buffer operations
 * 
 * This example shows how to:
 * - Create and manage ring buffers
 * - Monitor ring buffer statistics
 * - Handle basic ring buffer operations
 * - Proper cleanup and error handling
 */

#include <iostream>
#include <string>
#include <vector>
#include <chrono>
#include <thread>
#include <iomanip>
#include <random>
#include <atomic>

extern "C" {
#include "renoir.h"
}

class RenoirRingBufferExample {
private:
    RenoirRingBufferHandle ring_buffer = nullptr;
    
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

    void print_ring_stats(const RenoirRingStats& stats) {
        std::cout << "  Capacity: " << stats.capacity << " bytes" << std::endl;
        std::cout << "  Current Size: " << stats.size << " bytes" << std::endl;
        std::cout << "  Writes: " << stats.writes << std::endl;
        std::cout << "  Reads: " << stats.reads << std::endl;
        std::cout << "  Overwrites: " << stats.overwrites << std::endl;
        
        double utilization = 0.0;
        if (stats.capacity > 0) {
            utilization = (double)stats.size / stats.capacity;
        }
        std::cout << "  Utilization: " << std::fixed << std::setprecision(1) 
                  << (utilization * 100.0) << "%" << std::endl;
    }

public:
    void run_example() {
        print_header("Renoir Ring Buffer Example");
        
        try {
            create_ring_buffer();
            demonstrate_basic_operations();
            demonstrate_statistics();
            cleanup();
            
            std::cout << "\n✓ Ring buffer example completed successfully!" << std::endl;
            
        } catch (const std::exception& e) {
            std::cerr << "Exception: " << e.what() << std::endl;
            cleanup();
            exit(1);
        }
    }

private:
    void create_ring_buffer() {
        print_section("Creating Ring Buffer");

        // Create ring buffer with 64KB capacity
        size_t capacity = 64 * 1024;
        RenoirErrorCode error = renoir_ring_buffer_create(capacity, &ring_buffer);
        check_error(error, "creating ring buffer");

        std::cout << "✓ Ring buffer created:" << std::endl;
        std::cout << "  Capacity: " << capacity << " bytes" << std::endl;

        // Get initial statistics
        RenoirRingStats stats;
        error = renoir_ring_buffer_stats(ring_buffer, &stats);
        check_error(error, "getting initial ring buffer stats");

        std::cout << "\nInitial Ring Buffer Statistics:" << std::endl;
        print_ring_stats(stats);
    }

    void demonstrate_basic_operations() {
        print_section("Demonstrating Basic Operations");

        // Test if ring buffer is empty initially
        bool is_empty = false;
        RenoirErrorCode error = renoir_ring_buffer_is_empty(ring_buffer, &is_empty);
        check_error(error, "checking if ring buffer is empty");
        std::cout << "Ring buffer is empty: " << (is_empty ? "Yes" : "No") << std::endl;

        // Test if ring buffer is full initially
        bool is_full = false;
        error = renoir_ring_buffer_is_full(ring_buffer, &is_full);
        check_error(error, "checking if ring buffer is full");
        std::cout << "Ring buffer is full: " << (is_full ? "Yes" : "No") << std::endl;

        // Get available space
        size_t available_space = 0;
        error = renoir_ring_buffer_available_space(ring_buffer, &available_space);
        check_error(error, "getting available space");
        std::cout << "Available space: " << available_space << " bytes" << std::endl;

        // Get updated statistics
        RenoirRingStats stats;
        error = renoir_ring_buffer_stats(ring_buffer, &stats);
        check_error(error, "getting ring buffer stats");

        std::cout << "\nCurrent Ring Buffer Statistics:" << std::endl;
        print_ring_stats(stats);
    }

    void demonstrate_statistics() {
        print_section("Demonstrating Statistics Monitoring");

        std::cout << "Monitoring ring buffer statistics..." << std::endl;

        const int monitoring_cycles = 5;
        
        for (int cycle = 0; cycle < monitoring_cycles; ++cycle) {
            std::cout << "\n--- Cycle " << (cycle + 1) << " ---" << std::endl;

            // Check current state
            bool is_empty = false;
            bool is_full = false;
            size_t available_space = 0;

            RenoirErrorCode error = renoir_ring_buffer_is_empty(ring_buffer, &is_empty);
            if (error == Success) {
                std::cout << "Is Empty: " << (is_empty ? "Yes" : "No") << std::endl;
            }

            error = renoir_ring_buffer_is_full(ring_buffer, &is_full);
            if (error == Success) {
                std::cout << "Is Full: " << (is_full ? "Yes" : "No") << std::endl;
            }

            error = renoir_ring_buffer_available_space(ring_buffer, &available_space);
            if (error == Success) {
                std::cout << "Available Space: " << available_space << " bytes" << std::endl;
            }

            // Get statistics
            RenoirRingStats stats;
            error = renoir_ring_buffer_stats(ring_buffer, &stats);
            if (error == Success) {
                std::cout << "Statistics:" << std::endl;
                std::cout << "  Size: " << stats.size << " bytes" << std::endl;
                std::cout << "  Writes: " << stats.writes << std::endl;
                std::cout << "  Reads: " << stats.reads << std::endl;
                std::cout << "  Overwrites: " << stats.overwrites << std::endl;
            }

            // Brief pause between cycles
            if (cycle < monitoring_cycles - 1) {
                std::this_thread::sleep_for(std::chrono::milliseconds(200));
            }
        }

        // Final statistics
        RenoirRingStats final_stats;
        RenoirErrorCode error = renoir_ring_buffer_stats(ring_buffer, &final_stats);
        if (error == Success) {
            std::cout << "\nFinal Ring Buffer Statistics:" << std::endl;
            print_ring_stats(final_stats);
        }
    }

    void cleanup() {
        print_section("Cleanup");

        // Destroy ring buffer
        if (ring_buffer) {
            RenoirErrorCode error = renoir_ring_buffer_destroy(ring_buffer);
            if (error == Success) {
                std::cout << "✓ Ring buffer destroyed" << std::endl;
            }
            ring_buffer = nullptr;
        }
    }
};

int main() {
    std::cout << "Starting Renoir Ring Buffer Example..." << std::endl;
    
    RenoirRingBufferExample example;
    example.run_example();
    
    return 0;
}