/**
 * @file buffer_pool.cpp
 * @brief Buffer pool and buffer wrapper implementation
 */

#include "renoir.hpp"

namespace renoir {

RenoirBufferPoolConfig BufferPoolConfig::toC() const {
    RenoirBufferPoolConfig c = {};
    c.name = name.c_str();
    c.buffer_size = buffer_size;
    c.initial_count = initial_count;
    c.max_count = max_count;
    c.alignment = alignment;
    c.pre_allocate = pre_allocate;
    c.allocation_timeout_ms = static_cast<uint64_t>(allocation_timeout.count());
    return c;
}

BufferPool::BufferPool(RenoirManagerHandle manager, const std::string& region_name,
                       const BufferPoolConfig& config) {
    RenoirBufferPoolConfig c = config.toC();
    checkResult(renoir_buffer_pool_create(manager, region_name.c_str(), &c, &handle_),
                "create buffer pool");
}

BufferPool::BufferPool(BufferPool&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
}

BufferPool& BufferPool::operator=(BufferPool&& other) noexcept {
    if (this != &other) {
        destroy();
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

BufferPool::~BufferPool() {
    destroy();
}

void BufferPool::destroy() {
    if (handle_) {
        renoir_buffer_pool_destroy(handle_);
        handle_ = nullptr;
    }
}

Buffer BufferPool::getBuffer() {
    RenoirBufferHandle buffer = nullptr;
    checkResult(renoir_buffer_get(handle_, &buffer), "get buffer");
    return Buffer(buffer, handle_);
}

RenoirBufferPoolStats BufferPool::stats() const {
    RenoirBufferPoolStats raw = {};
    checkResult(renoir_buffer_pool_stats(handle_, &raw), "buffer pool stats");
    return raw;
}

size_t BufferPool::availableCount() const {
    size_t count = 0;
    checkResult(renoir_buffer_pool_available_count(handle_, &count), "available buffer count");
    return count;
}

size_t BufferPool::allocatedCount() const {
    size_t count = 0;
    checkResult(renoir_buffer_pool_allocated_count(handle_, &count), "allocated buffer count");
    return count;
}

bool BufferPool::canExpand() const {
    bool result = false;
    checkResult(renoir_buffer_pool_can_expand(handle_, &result), "buffer pool can expand");
    return result;
}

size_t BufferPool::shrink(size_t target_available) {
    size_t shrunk = 0;
    checkResult(renoir_buffer_pool_shrink(handle_, target_available, &shrunk),
                "shrink buffer pool");
    return shrunk;
}

Buffer::Buffer(RenoirBufferHandle handle, RenoirBufferPoolHandle pool)
    : handle_(handle), pool_(pool) {}

Buffer::Buffer(Buffer&& other) noexcept : handle_(other.handle_), pool_(other.pool_) {
    other.handle_ = nullptr;
    other.pool_ = nullptr;
}

Buffer& Buffer::operator=(Buffer&& other) noexcept {
    if (this != &other) {
        release();
        handle_ = other.handle_;
        pool_ = other.pool_;
        other.handle_ = nullptr;
        other.pool_ = nullptr;
    }
    return *this;
}

Buffer::~Buffer() {
    release();
}

void Buffer::release() {
    if (handle_ && pool_) {
        renoir_buffer_return(pool_, handle_);
    }
    handle_ = nullptr;
    pool_ = nullptr;
}

RenoirBufferInfo Buffer::info() const {
    RenoirBufferInfo raw = {};
    checkResult(renoir_buffer_info(handle_, &raw), "get buffer info");
    return raw;
}

const uint8_t* Buffer::data() const {
    const uint8_t* ptr = nullptr;
    size_t len = 0;
    checkResult(renoir_buffer_data(handle_, &ptr, &len), "get buffer data");
    return ptr;
}

uint8_t* Buffer::data() {
    uint8_t* ptr = nullptr;
    size_t len = 0;
    checkResult(renoir_buffer_data_mut(handle_, &ptr, &len), "get buffer data");
    return ptr;
}

size_t Buffer::size() const {
    return info().size;
}

size_t Buffer::capacity() const {
    return info().capacity;
}

uint64_t Buffer::sequence() const {
    uint64_t seq = 0;
    checkResult(renoir_buffer_sequence(handle_, &seq), "get buffer sequence");
    return seq;
}

void Buffer::resize(size_t new_size) {
    checkResult(renoir_buffer_resize(handle_, new_size), "resize buffer");
}

void Buffer::clear() {
    checkResult(renoir_buffer_clear(handle_), "clear buffer");
}

void Buffer::writeBytes(const std::vector<uint8_t>& bytes) {
    if (bytes.size() > capacity()) {
        throw Exception(RenoirErrorCode::RenoirErrorCode_InsufficientSpace, "bytes too large for buffer");
    }
    uint8_t* dst = data();
    for (size_t i = 0; i < bytes.size(); ++i) {
        dst[i] = bytes[i];
    }
    resize(bytes.size());
}

std::vector<uint8_t> Buffer::readBytes() const {
    size_t len = size();
    const uint8_t* src = data();
    return std::vector<uint8_t>(src, src + len);
}

std::string Buffer::readString() const {
    std::vector<uint8_t> bytes = readBytes();
    return std::string(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

} // namespace renoir
