/**
 * @file c_example.c
 * @brief Basic C example using Renoir C API
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "renoir.h"

int main() {
    printf("Renoir C API Example\n");
    printf("===================\n");
    
    // Initialize Renoir
    RenoirErrorCode result = renoir_init();
    if (result != RENOIR_SUCCESS) {
        fprintf(stderr, "Failed to initialize Renoir: %d\n", result);
        return 1;
    }
    
    printf("Renoir initialized successfully\n");
    
    // Print version information
    printf("Version: %u.%u.%u\n", 
           renoir_version_major(), 
           renoir_version_minor(), 
           renoir_version_patch());
    
    char* version_str = renoir_version_string();
    if (version_str) {
        printf("Version string: %s\n\n", version_str);
        renoir_free_string(version_str);
    }
    
    // Create shared memory manager
    RenoirManagerHandle manager = renoir_manager_create();
    if (!manager) {
        fprintf(stderr, "Failed to create manager\n");
        return 1;
    }
    
    printf("Created shared memory manager\n");
    
    // Configure and create region
    RenoirRegionConfig region_config = {
        .name = "c_example_region",
        .size = 1024 * 1024,  // 1MB
        .backing_type = 0,    // FileBacked
        .file_path = "/tmp/renoir_c_example",
        .create = true,
        .permissions = 0644
    };
    
    RenoirRegionHandle region;
    result = renoir_region_create(manager, &region_config, &region);
    if (result != RENOIR_SUCCESS) {
        fprintf(stderr, "Failed to create region: %d\n", result);
        renoir_manager_destroy(manager);
        return 1;
    }
    
    printf("Created shared memory region: %s\n", region_config.name);
    
    // Configure and create buffer pool
    RenoirBufferPoolConfig pool_config = {
        .name = "c_example_pool",
        .buffer_size = 4096,
        .initial_count = 10,
        .max_count = 50,
        .alignment = 64,
        .pre_allocate = true,
        .allocation_timeout_ms = 100
    };
    
    RenoirBufferPoolHandle pool;
    result = renoir_buffer_pool_create(region, &pool_config, &pool);
    if (result != RENOIR_SUCCESS) {
        fprintf(stderr, "Failed to create buffer pool: %d\n", result);
        renoir_manager_destroy(manager);
        return 1;
    }
    
    printf("Created buffer pool: %s\n", pool_config.name);
    
    // Test buffer operations
    printf("\nTesting buffer operations...\n");
    
    const int num_operations = 20;
    RenoirBufferHandle buffers[20];
    
    // Allocate buffers and write data
    for (int i = 0; i < num_operations; i++) {
        result = renoir_buffer_get(pool, &buffers[i]);
        if (result != RENOIR_SUCCESS) {
            fprintf(stderr, "Failed to get buffer %d: %d\n", i, result);
            break;
        }
        
        // Get buffer info and write data
        RenoirBufferInfo info;
        result = renoir_buffer_info(buffers[i], &info);
        if (result != RENOIR_SUCCESS) {
            fprintf(stderr, "Failed to get buffer info: %d\n", result);
            continue;
        }
        
        // Write test data
        char test_data[256];
        snprintf(test_data, sizeof(test_data), "C API test data item #%d", i);
        size_t data_len = strlen(test_data);
        
        if (data_len <= info.capacity) {
            memcpy(info.data, test_data, data_len);
            renoir_buffer_resize(buffers[i], data_len);
            renoir_buffer_set_sequence(buffers[i], i);
        }
        
        printf("  Buffer %d: wrote %zu bytes, sequence %d\n", i, data_len, i);
    }
    
    // Get pool statistics
    RenoirBufferPoolStats stats;
    result = renoir_buffer_pool_stats(pool, &stats);
    if (result == RENOIR_SUCCESS) {
        printf("\nBuffer Pool Statistics:\n");
        printf("  Total allocated: %zu\n", stats.total_allocated);
        printf("  Currently in use: %zu\n", stats.currently_in_use);
        printf("  Peak usage: %zu\n", stats.peak_usage);
        printf("  Success rate: %.2f%%\n", stats.success_rate * 100.0);
        printf("  Utilization: %.2f%%\n", stats.utilization * 100.0);
    }
    
    // Return all buffers to pool
    printf("\nReturning buffers to pool...\n");
    for (int i = 0; i < num_operations; i++) {
        if (buffers[i]) {
            // Read data before returning
            RenoirBufferInfo info;
            if (renoir_buffer_info(buffers[i], &info) == RENOIR_SUCCESS) {
                printf("  Buffer %d contained: %.*s\n", 
                       i, (int)info.size, (char*)info.data);
            }
            
            renoir_buffer_return(pool, buffers[i]);
        }
    }
    
    // Final statistics
    result = renoir_buffer_pool_stats(pool, &stats);
    if (result == RENOIR_SUCCESS) {
        printf("\nFinal Statistics:\n");
        printf("  Currently in use: %zu\n", stats.currently_in_use);
        printf("  Total allocations: %llu\n", 
               (unsigned long long)stats.total_allocations);
        printf("  Total deallocations: %llu\n", 
               (unsigned long long)stats.total_deallocations);
    }
    
    // Cleanup
    renoir_buffer_pool_destroy(pool);
    renoir_manager_destroy(manager);
    
    printf("\nC example completed successfully!\n");
    
    return 0;
}