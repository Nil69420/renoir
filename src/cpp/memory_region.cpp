/**
 * @file memory_region.cpp
 * @brief Shared memory manager and region wrapper implementation
 */

#include "renoir.hpp"

#include <cstring>

namespace renoir {

RenoirRegionConfig RegionConfig::toC() const {
    RenoirRegionConfig c = {};
    c.name = name.c_str();
    c.size = size;
    c.backing_type = use_memfd ? 1 : 0;
    c.file_path = file_path.empty() ? nullptr : file_path.c_str();
    c.create = create;
    c.permissions = permissions;
    return c;
}

SharedMemoryManager::SharedMemoryManager() {
    handle_ = renoir_manager_create();
    if (!handle_) {
        throw Exception(RenoirErrorCode::RenoirErrorCode_OutOfMemory, "create shared memory manager");
    }
}

SharedMemoryManager::SharedMemoryManager(const RegionConfig& control_config) {
    RenoirRegionConfig c = control_config.toC();
    handle_ = renoir_manager_create_with_control(&c);
    if (!handle_) {
        throw Exception(RenoirErrorCode::RenoirErrorCode_OutOfMemory, "create shared memory manager with control");
    }
}

SharedMemoryManager& SharedMemoryManager::operator=(SharedMemoryManager&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_manager_destroy(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

SharedMemoryManager::~SharedMemoryManager() {
    if (handle_) {
        renoir_manager_destroy(handle_);
    }
}

SharedMemoryRegion SharedMemoryManager::createRegion(const RegionConfig& config) {
    RenoirRegionConfig c = config.toC();
    RenoirRegionHandle region = nullptr;
    checkResult(renoir_region_create(handle_, &c, &region), "create shared memory region");
    return SharedMemoryRegion(region);
}

SharedMemoryRegion SharedMemoryManager::getRegion(const std::string& name) {
    RenoirRegionHandle region = nullptr;
    checkResult(renoir_region_get(handle_, name.c_str(), &region), "get shared memory region");
    return SharedMemoryRegion(region);
}

SharedMemoryRegion SharedMemoryManager::openRegion(const std::string& name) {
    RenoirRegionHandle region = nullptr;
    checkResult(renoir_region_open(handle_, name.c_str(), &region), "open shared memory region");
    return SharedMemoryRegion(region);
}

BufferPool SharedMemoryManager::createBufferPool(const std::string& region_name,
                                                 const BufferPoolConfig& config) {
    RenoirBufferPoolConfig c = config.toC();
    RenoirBufferPoolHandle pool = nullptr;
    checkResult(renoir_buffer_pool_create(handle_, region_name.c_str(), &c, &pool),
                "create buffer pool");
    return BufferPool(pool);
}

void SharedMemoryManager::removeRegion(const std::string& name) {
    checkResult(renoir_region_destroy(handle_, name.c_str()), "remove shared memory region");
}

std::vector<std::string> SharedMemoryManager::listRegions() const {
    char** names = nullptr;
    size_t count = 0;
    checkResult(renoir_manager_list_regions(handle_, &names, &count), "list regions");
    std::vector<std::string> result;
    result.reserve(count);
    for (size_t i = 0; i < count; ++i) {
        result.emplace_back(names[i] ? names[i] : "");
    }
    renoir_free_region_names(names, count);
    return result;
}

size_t SharedMemoryManager::totalMemoryUsage() const {
    size_t total = 0;
    checkResult(renoir_manager_total_memory_usage(handle_, &total), "total memory usage");
    return total;
}

size_t SharedMemoryManager::regionCount() const {
    size_t count = 0;
    checkResult(renoir_manager_region_count(handle_, &count), "region count");
    return count;
}

void SharedMemoryManager::flushAll() {
    checkResult(renoir_manager_flush_all(handle_), "flush all regions");
}

void SharedMemoryManager::clearAll() {
    checkResult(renoir_manager_clear_all(handle_), "clear all regions");
}

SharedMemoryRegion& SharedMemoryRegion::operator=(SharedMemoryRegion&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_region_close(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

SharedMemoryRegion::~SharedMemoryRegion() {
    if (handle_) {
        renoir_region_close(handle_);
    }
}

size_t SharedMemoryRegion::size() const {
    return renoir_region_size(handle_);
}

std::string SharedMemoryRegion::name() const {
    char* raw = renoir_region_name(handle_);
    if (!raw) {
        return std::string();
    }
    std::string result(raw);
    renoir_free_string(raw);
    return result;
}

RenoirRegionStats SharedMemoryRegion::stats() const {
    RenoirRegionStats stats = {};
    checkResult(renoir_region_stats(handle_, &stats), "region stats");
    return stats;
}

} // namespace renoir
