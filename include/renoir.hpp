/**
 * @file renoir.hpp
 * @brief C++ wrapper for Renoir Shared Memory Manager
 */

#ifndef RENOIR_HPP
#define RENOIR_HPP

#include "renoir.h"
#include <memory>
#include <string>
#include <vector>
#include <stdexcept>
#include <chrono>
#include <cstring>

namespace renoir {

/**
 * @brief Exception class for Renoir errors
 */
class Exception : public std::runtime_error {
public:
    explicit Exception(RenoirErrorCode code, const std::string& message = "")
        : std::runtime_error(formatMessage(code, message)), error_code_(code) {}
    
    RenoirErrorCode code() const noexcept { return error_code_; }

private:
    RenoirErrorCode error_code_;
    
    static std::string formatMessage(RenoirErrorCode code, const std::string& message) {
        std::string result = "Renoir error (" + std::to_string(static_cast<int>(code)) + ")";
        if (!message.empty()) {
            result += ": " + message;
        }
        return result;
    }
};

/**
 * @brief RAII wrapper for checking Renoir error codes
 */
inline void checkResult(RenoirErrorCode result, const std::string& operation = "") {
    if (result != RENOIR_SUCCESS) {
        throw Exception(result, operation);
    }
}

/**
 * @brief Buffer information structure
 */
struct BufferInfo {
    uint8_t* data;
    size_t size;
    size_t capacity;
    uint64_t sequence;
    
    BufferInfo() : data(nullptr), size(0), capacity(0), sequence(0) {}
    
    BufferInfo(const RenoirBufferInfo& info) 
        : data(info.data), size(info.size), capacity(info.capacity), sequence(info.sequence) {}
};

/**
 * @brief Buffer pool statistics
 */
struct BufferPoolStats {
    size_t total_allocated;
    size_t currently_in_use;
    size_t peak_usage;
    uint64_t total_allocations;
    uint64_t total_deallocations;
    uint64_t allocation_failures;
    double success_rate;
    double utilization;
    
    BufferPoolStats(const RenoirBufferPoolStats& stats)
        : total_allocated(stats.total_allocated)
        , currently_in_use(stats.currently_in_use)
        , peak_usage(stats.peak_usage)
        , total_allocations(stats.total_allocations)
        , total_deallocations(stats.total_deallocations)
        , allocation_failures(stats.allocation_failures)
        , success_rate(stats.success_rate)
        , utilization(stats.utilization) {}
};

// Forward declarations
class SharedMemoryManager;
class SharedMemoryRegion;
class BufferPool;

/**
 * @brief RAII wrapper for Renoir buffers
 */
class Buffer {
public:
    Buffer() = delete;
    Buffer(const Buffer&) = delete;
    Buffer& operator=(const Buffer&) = delete;
    
    // Move constructor
    Buffer(Buffer&& other) noexcept 
        : handle_(other.handle_), pool_(other.pool_), info_(other.info_) {
        other.handle_ = nullptr;
        other.pool_ = nullptr;
    }
    
    // Move assignment
    Buffer& operator=(Buffer&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            pool_ = other.pool_;
            info_ = other.info_;
            other.handle_ = nullptr;
            other.pool_ = nullptr;
        }
        return *this;
    }
    
    ~Buffer() {
        reset();
    }
    
    // Accessors
    uint8_t* data() { return info_.data; }
    const uint8_t* data() const { return info_.data; }
    size_t size() const { return info_.size; }
    size_t capacity() const { return info_.capacity; }
    uint64_t sequence() const { return info_.sequence; }
    
    // Operations
    void resize(size_t new_size) {
        checkResult(renoir_buffer_resize(handle_, new_size), "resize buffer");
        updateInfo();
    }
    
    void setSequence(uint64_t seq) {
        checkResult(renoir_buffer_set_sequence(handle_, seq), "set buffer sequence");
        updateInfo();
    }
    
    // Convenience methods
    template<typename T>
    T* as() { return reinterpret_cast<T*>(data()); }
    
    template<typename T>
    const T* as() const { return reinterpret_cast<const T*>(data()); }
    
    void writeString(const std::string& str) {
        if (str.size() > capacity()) {
            throw Exception(RENOIR_INVALID_PARAMETER, "String too large for buffer");
        }
        std::memcpy(data(), str.c_str(), str.size());
        resize(str.size());
    }
    
    std::string readString() const {
        return std::string(reinterpret_cast<const char*>(data()), size());
    }

private:
    friend class BufferPool;
    
    Buffer(RenoirBufferHandle handle, RenoirBufferPoolHandle pool) 
        : handle_(handle), pool_(pool) {
        updateInfo();
    }
    
    void reset() {
        if (handle_ && pool_) {
            renoir_buffer_return(pool_, handle_);
        }
        handle_ = nullptr;
        pool_ = nullptr;
    }
    
    void updateInfo() {
        if (handle_) {
            RenoirBufferInfo raw_info;
            checkResult(renoir_buffer_info(handle_, &raw_info), "get buffer info");
            info_ = BufferInfo(raw_info);
        }
    }
    
    RenoirBufferHandle handle_ = nullptr;
    RenoirBufferPoolHandle pool_ = nullptr;
    BufferInfo info_{};
};

/**
 * @brief Configuration for buffer pools
 */
struct BufferPoolConfig {
    std::string name;
    size_t buffer_size = 4096;
    size_t initial_count = 16;
    size_t max_count = 1024;
    size_t alignment = 64;
    bool pre_allocate = true;
    std::chrono::milliseconds allocation_timeout{100};
};

/**
 * @brief RAII wrapper for Renoir buffer pools
 */
class BufferPool {
public:
    BufferPool() = delete;
    BufferPool(const BufferPool&) = delete;
    BufferPool& operator=(const BufferPool&) = delete;
    
    BufferPool(BufferPool&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }
    
    BufferPool& operator=(BufferPool&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }
    
    ~BufferPool() {
        reset();
    }
    
    // Create buffer pool
    BufferPool(RenoirRegionHandle region, const BufferPoolConfig& config) {
        RenoirBufferPoolConfig c_config = {};
        c_config.name = config.name.c_str();
        c_config.buffer_size = config.buffer_size;
        c_config.initial_count = config.initial_count;
        c_config.max_count = config.max_count;
        c_config.alignment = config.alignment;
        c_config.pre_allocate = config.pre_allocate;
        c_config.allocation_timeout_ms = config.allocation_timeout.count();
        
        checkResult(renoir_buffer_pool_create(region, &c_config, &handle_), 
                   "create buffer pool");
    }
    
    // Get a buffer from the pool
    Buffer getBuffer() {
        RenoirBufferHandle buffer_handle;
        checkResult(renoir_buffer_get(handle_, &buffer_handle), "get buffer");
        return Buffer(buffer_handle, handle_);
    }
    
    // Get pool statistics
    BufferPoolStats getStats() const {
        RenoirBufferPoolStats raw_stats;
        checkResult(renoir_buffer_pool_stats(handle_, &raw_stats), "get pool stats");
        return BufferPoolStats(raw_stats);
    }
    
    // Reset the pool
    void reset() {
        if (handle_) {
            renoir_buffer_pool_destroy(handle_);
            handle_ = nullptr;
        }
    }

private:
    friend class SharedMemoryRegion;
    
    BufferPool(RenoirBufferPoolHandle handle) : handle_(handle) {}
    
    RenoirBufferPoolHandle handle_ = nullptr;
};

/**
 * @brief Configuration for shared memory regions
 */
struct RegionConfig {
    std::string name;
    size_t size;
    bool use_memfd = false;  // Use memfd instead of file-backed
    std::string file_path;   // Optional file path
    bool create = true;
    uint32_t permissions = 0644;
};

/**
 * @brief RAII wrapper for shared memory regions
 */
class SharedMemoryRegion {
public:
    SharedMemoryRegion() = delete;
    SharedMemoryRegion(const SharedMemoryRegion&) = delete;
    SharedMemoryRegion& operator=(const SharedMemoryRegion&) = delete;
    
    SharedMemoryRegion(SharedMemoryRegion&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }
    
    SharedMemoryRegion& operator=(SharedMemoryRegion&& other) noexcept {
        if (this != &other) {
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }
    
    // Create region from manager
    SharedMemoryRegion(RenoirManagerHandle manager, const RegionConfig& config) {
        RenoirRegionConfig c_config = {};
        c_config.name = config.name.c_str();
        c_config.size = config.size;
        c_config.backing_type = config.use_memfd ? 1 : 0;
        c_config.file_path = config.file_path.empty() ? nullptr : config.file_path.c_str();
        c_config.create = config.create;
        c_config.permissions = config.permissions;
        
        checkResult(renoir_region_create(manager, &c_config, &handle_), 
                   "create shared memory region");
    }
    
    // Get existing region from manager
    SharedMemoryRegion(RenoirManagerHandle manager, const std::string& name) {
        checkResult(renoir_region_get(manager, name.c_str(), &handle_), 
                   "get shared memory region");
    }
    
    // Create buffer pool
    BufferPool createBufferPool(const BufferPoolConfig& config) {
        return BufferPool(handle_, config);
    }
    
    // Get raw pointer to memory
    std::pair<void*, size_t> getMemory() {
        void* ptr;
        size_t size;
        checkResult(renoir_region_get_ptr(handle_, &ptr, &size), "get region memory");
        return {ptr, size};
    }
    
    // Flush changes to storage
    void flush() {
        checkResult(renoir_region_flush(handle_), "flush region");
    }
    
    RenoirRegionHandle handle() const { return handle_; }

private:
    friend class SharedMemoryManager;
    
    SharedMemoryRegion(RenoirRegionHandle handle) : handle_(handle) {}
    
    RenoirRegionHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for shared memory manager
 */
class SharedMemoryManager {
public:
    SharedMemoryManager() {
        handle_ = renoir_manager_create();
        if (!handle_) {
            throw Exception(RENOIR_OUT_OF_MEMORY, "create shared memory manager");
        }
    }
    
    SharedMemoryManager(const SharedMemoryManager&) = delete;
    SharedMemoryManager& operator=(const SharedMemoryManager&) = delete;
    
    SharedMemoryManager(SharedMemoryManager&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }
    
    SharedMemoryManager& operator=(SharedMemoryManager&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }
    
    ~SharedMemoryManager() {
        reset();
    }
    
    // Create a new shared memory region
    SharedMemoryRegion createRegion(const RegionConfig& config) {
        return SharedMemoryRegion(handle_, config);
    }
    
    // Get an existing shared memory region
    SharedMemoryRegion getRegion(const std::string& name) {
        return SharedMemoryRegion(handle_, name);
    }
    
    // Remove a region
    void removeRegion(const std::string& name) {
        checkResult(renoir_region_remove(handle_, name.c_str()), "remove region");
    }

private:
    void reset() {
        if (handle_) {
            renoir_manager_destroy(handle_);
            handle_ = nullptr;
        }
    }
    
    RenoirManagerHandle handle_ = nullptr;
};

/**
 * @brief Initialize the Renoir library
 */
inline void initialize() {
    checkResult(renoir_init(), "initialize Renoir library");
}

/**
 * @brief Get library version information
 */
inline std::tuple<uint32_t, uint32_t, uint32_t> getVersion() {
    return {renoir_version_major(), renoir_version_minor(), renoir_version_patch()};
}

/**
 * @brief Get library version string
 */
inline std::string getVersionString() {
    char* version_str = renoir_version_string();
    if (!version_str) {
        return "unknown";
    }
    
    std::string result(version_str);
    renoir_free_string(version_str);
    return result;
}

} // namespace renoir

#endif // RENOIR_HPP