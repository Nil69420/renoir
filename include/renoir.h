/**
 * @file renoir.h
 * @brief C API for Renoir Shared Memory Manager
 * 
 * This header provides a C interface to the Renoir shared memory library,
 * enabling integration with C++ applications and ROS2 nodes.
 */

#ifndef RENOIR_H
#define RENOIR_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

/* Opaque handle types */
typedef void* RenoirManagerHandle;
typedef void* RenoirRegionHandle;
typedef void* RenoirBufferPoolHandle;
typedef void* RenoirBufferHandle;
typedef void* RenoirAllocatorHandle;
typedef void* RenoirRingBufferHandle;
typedef void* RenoirProducerHandle;
typedef void* RenoirConsumerHandle;

/* Error codes */
typedef enum {
    RENOIR_SUCCESS = 0,
    RENOIR_INVALID_PARAMETER = 1,
    RENOIR_OUT_OF_MEMORY = 2,
    RENOIR_REGION_NOT_FOUND = 3,
    RENOIR_REGION_EXISTS = 4,
    RENOIR_INSUFFICIENT_SPACE = 5,
    RENOIR_BUFFER_FULL = 6,
    RENOIR_BUFFER_EMPTY = 7,
    RENOIR_IO_ERROR = 8,
    RENOIR_SERIALIZATION_ERROR = 9,
    RENOIR_CONCURRENCY_ERROR = 10,
    RENOIR_ALIGNMENT_ERROR = 11,
    RENOIR_VERSION_MISMATCH = 12,
    RENOIR_PLATFORM_ERROR = 13,
    RENOIR_UNKNOWN_ERROR = 99
} RenoirErrorCode;

/* Configuration structures */
typedef struct {
    const char* name;
    size_t size;
    uint32_t backing_type;  /* 0 = FileBacked, 1 = MemFd */
    const char* file_path;  /* Optional, can be NULL */
    bool create;
    uint32_t permissions;
} RenoirRegionConfig;

typedef struct {
    const char* name;
    size_t buffer_size;
    size_t initial_count;
    size_t max_count;
    size_t alignment;
    bool pre_allocate;
    uint64_t allocation_timeout_ms;  /* 0 = no timeout */
} RenoirBufferPoolConfig;

/* Information structures */
typedef struct {
    uint8_t* data;
    size_t size;
    size_t capacity;
    uint64_t sequence;
} RenoirBufferInfo;

typedef struct {
    size_t total_size;
    size_t used_size;
    size_t available_size;
} RenoirAllocatorStats;

typedef struct {
    size_t total_allocated;
    size_t currently_in_use;
    size_t peak_usage;
    uint64_t total_allocations;
    uint64_t total_deallocations;
    uint64_t allocation_failures;
    double success_rate;
    double utilization;
} RenoirBufferPoolStats;

/* Library initialization */
RenoirErrorCode renoir_init(void);

/* Shared Memory Manager API */
RenoirManagerHandle renoir_manager_create(void);
RenoirManagerHandle renoir_manager_create_with_control(const RenoirRegionConfig* config);
RenoirErrorCode renoir_manager_destroy(RenoirManagerHandle manager);

/* Shared Memory Region API */
RenoirErrorCode renoir_region_create(
    RenoirManagerHandle manager,
    const RenoirRegionConfig* config,
    RenoirRegionHandle* region
);

RenoirErrorCode renoir_region_get(
    RenoirManagerHandle manager,
    const char* name,
    RenoirRegionHandle* region
);

RenoirErrorCode renoir_region_remove(
    RenoirManagerHandle manager,
    const char* name
);

RenoirErrorCode renoir_region_info(
    RenoirRegionHandle region,
    const char** name,
    size_t* size,
    uint32_t* backing_type
);

RenoirErrorCode renoir_region_get_ptr(
    RenoirRegionHandle region,
    void** ptr,
    size_t* size
);

RenoirErrorCode renoir_region_flush(RenoirRegionHandle region);

/* Buffer Pool API */
RenoirErrorCode renoir_buffer_pool_create(
    RenoirRegionHandle region,
    const RenoirBufferPoolConfig* config,
    RenoirBufferPoolHandle* pool
);

RenoirErrorCode renoir_buffer_pool_destroy(RenoirBufferPoolHandle pool);

RenoirErrorCode renoir_buffer_get(
    RenoirBufferPoolHandle pool,
    RenoirBufferHandle* buffer
);

RenoirErrorCode renoir_buffer_return(
    RenoirBufferPoolHandle pool,
    RenoirBufferHandle buffer
);

RenoirErrorCode renoir_buffer_info(
    RenoirBufferHandle buffer,
    RenoirBufferInfo* info
);

RenoirErrorCode renoir_buffer_resize(
    RenoirBufferHandle buffer,
    size_t new_size
);

RenoirErrorCode renoir_buffer_set_sequence(
    RenoirBufferHandle buffer,
    uint64_t sequence
);

RenoirErrorCode renoir_buffer_pool_stats(
    RenoirBufferPoolHandle pool,
    RenoirBufferPoolStats* stats
);

RenoirErrorCode renoir_buffer_pool_reset(RenoirBufferPoolHandle pool);

/* Allocator API */
RenoirErrorCode renoir_allocator_create_bump(
    RenoirRegionHandle region,
    RenoirAllocatorHandle* allocator
);

RenoirErrorCode renoir_allocator_create_pool(
    RenoirRegionHandle region,
    size_t block_size,
    RenoirAllocatorHandle* allocator
);

RenoirErrorCode renoir_allocator_destroy(RenoirAllocatorHandle allocator);

RenoirErrorCode renoir_allocator_allocate(
    RenoirAllocatorHandle allocator,
    size_t size,
    size_t align,
    void** ptr
);

RenoirErrorCode renoir_allocator_deallocate(
    RenoirAllocatorHandle allocator,
    void* ptr,
    size_t size,
    size_t align
);

RenoirErrorCode renoir_allocator_stats(
    RenoirAllocatorHandle allocator,
    RenoirAllocatorStats* stats
);

RenoirErrorCode renoir_allocator_reset(RenoirAllocatorHandle allocator);

/* Ring Buffer API */
RenoirErrorCode renoir_ringbuffer_create(
    size_t capacity,
    size_t element_size,
    RenoirRingBufferHandle* ringbuf
);

RenoirErrorCode renoir_ringbuffer_destroy(RenoirRingBufferHandle ringbuf);

RenoirErrorCode renoir_ringbuffer_producer(
    RenoirRingBufferHandle ringbuf,
    RenoirProducerHandle* producer
);

RenoirErrorCode renoir_ringbuffer_consumer(
    RenoirRingBufferHandle ringbuf,
    RenoirConsumerHandle* consumer
);

RenoirErrorCode renoir_producer_push(
    RenoirProducerHandle producer,
    const void* data,
    size_t size
);

RenoirErrorCode renoir_producer_try_push(
    RenoirProducerHandle producer,
    const void* data,
    size_t size
);

RenoirErrorCode renoir_consumer_pop(
    RenoirConsumerHandle consumer,
    void* data,
    size_t size
);

RenoirErrorCode renoir_consumer_try_pop(
    RenoirConsumerHandle consumer,
    void* data,
    size_t size
);

RenoirErrorCode renoir_ringbuffer_info(
    RenoirRingBufferHandle ringbuf,
    size_t* capacity,
    size_t* length,
    bool* is_empty,
    bool* is_full
);

/* Utility functions */
void renoir_free_string(char* ptr);

/* Version information */
uint32_t renoir_version_major(void);
uint32_t renoir_version_minor(void);
uint32_t renoir_version_patch(void);
char* renoir_version_string(void);

/* Error handling utilities */
const char* renoir_error_string(RenoirErrorCode error);

#ifdef __cplusplus
}
#endif

#endif /* RENOIR_H */