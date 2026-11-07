/**
 * @file renoir.hpp
 * @brief C++ RAII wrapper for the Renoir C API
 */

#ifndef RENOIR_HPP
#define RENOIR_HPP

#include "renoir.h"
#include <chrono>
#include <cstdint>
#include <memory>
#include <stdexcept>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

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
 * @brief Check a Renoir error code and throw on failure
 */
inline void checkResult(RenoirErrorCode result, const std::string& operation = "") {
    if (result != RenoirErrorCode::RenoirErrorCode_Success) {
        throw Exception(result, operation);
    }
}

/**
 * @brief C++-friendly region configuration
 */
struct RegionConfig {
    std::string name;
    size_t size = 0;
    bool use_memfd = false;
    std::string file_path;
    bool create = true;
    uint32_t permissions = 0644;

    RenoirRegionConfig toC() const;
};

/**
 * @brief C++-friendly buffer pool configuration
 */
struct BufferPoolConfig {
    std::string name;
    size_t buffer_size = 4096;
    size_t initial_count = 16;
    size_t max_count = 1024;
    size_t alignment = 64;
    bool pre_allocate = true;
    std::chrono::milliseconds allocation_timeout{100};

    RenoirBufferPoolConfig toC() const;
};

/**
 * @brief C++-friendly topic registration options
 */
struct TopicOptions {
    uint32_t pattern = 0;
    size_t ring_capacity = 16;
    size_t max_payload_size = 1024;
    bool use_shared_pool = false;
    size_t shared_pool_threshold = 4096;
    bool enable_notifications = true;

    RenoirTopicOptions toC() const;
};

/**
 * @brief RAII wrapper for a received message
 */
class ReceivedMessage {
public:
    ReceivedMessage(RenoirReceivedMessage msg, RenoirReceivedMessageHandle handle)
        : msg_(msg), handle_(handle) {}
    ReceivedMessage(const ReceivedMessage&) = delete;
    ReceivedMessage& operator=(const ReceivedMessage&) = delete;

    ReceivedMessage(ReceivedMessage&& other) noexcept : msg_(other.msg_), handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    ReceivedMessage& operator=(ReceivedMessage&& other) noexcept {
        if (this != &other) {
            release();
            msg_ = other.msg_;
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~ReceivedMessage() { release(); }

    const uint8_t* data() const { return msg_.payload_ptr; }
    size_t size() const { return msg_.payload_len; }
    RenoirSequenceNumber sequence() const { return msg_.metadata.sequence_number; }
    uint64_t timestampNs() const { return msg_.metadata.timestamp_ns; }

private:
    void release();

    RenoirReceivedMessage msg_;
    RenoirReceivedMessageHandle handle_;
};

class SharedMemoryManager;
class SharedMemoryRegion;
class BufferPool;
class RingBuffer;
class TopicManager;
class Publisher;
class Subscriber;

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

    SharedMemoryRegion& operator=(SharedMemoryRegion&& other) noexcept;

    ~SharedMemoryRegion();

    size_t size() const;
    std::string name() const;
    RenoirRegionStats stats() const;

    RenoirRegionHandle handle() const { return handle_; }

private:
    friend class SharedMemoryManager;
    explicit SharedMemoryRegion(RenoirRegionHandle handle) : handle_(handle) {}

    RenoirRegionHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for shared memory managers
 */
class SharedMemoryManager {
public:
    SharedMemoryManager();
    explicit SharedMemoryManager(const RegionConfig& control_config);
    SharedMemoryManager(const SharedMemoryManager&) = delete;
    SharedMemoryManager& operator=(const SharedMemoryManager&) = delete;

    SharedMemoryManager(SharedMemoryManager&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    SharedMemoryManager& operator=(SharedMemoryManager&& other) noexcept;

    ~SharedMemoryManager();

    SharedMemoryRegion createRegion(const RegionConfig& config);
    SharedMemoryRegion getRegion(const std::string& name);
    SharedMemoryRegion openRegion(const std::string& name);
    BufferPool createBufferPool(const std::string& region_name, const BufferPoolConfig& config);
    void removeRegion(const std::string& name);
    std::vector<std::string> listRegions() const;
    size_t totalMemoryUsage() const;
    size_t regionCount() const;
    void flushAll();
    void clearAll();

    RenoirManagerHandle handle() const { return handle_; }

private:
    RenoirManagerHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for buffers obtained from a pool
 */
class Buffer {
public:
    Buffer() = delete;
    Buffer(const Buffer&) = delete;
    Buffer& operator=(const Buffer&) = delete;

    Buffer(Buffer&& other) noexcept;
    Buffer& operator=(Buffer&& other) noexcept;
    ~Buffer();

    const uint8_t* data() const;
    uint8_t* data();
    size_t size() const;
    size_t capacity() const;
    uint64_t sequence() const;
    void resize(size_t new_size);
    void clear();
    void writeBytes(const std::vector<uint8_t>& bytes);
    std::vector<uint8_t> readBytes() const;
    std::string readString() const;

private:
    friend class BufferPool;
    Buffer(RenoirBufferHandle handle, RenoirBufferPoolHandle pool);

    void release();
    RenoirBufferInfo info() const;

    RenoirBufferHandle handle_ = nullptr;
    RenoirBufferPoolHandle pool_ = nullptr;
};

/**
 * @brief RAII wrapper for buffer pools
 */
class BufferPool {
public:
    BufferPool() = delete;
    BufferPool(const BufferPool&) = delete;
    BufferPool& operator=(const BufferPool&) = delete;

    BufferPool(BufferPool&& other) noexcept;
    BufferPool& operator=(BufferPool&& other) noexcept;
    ~BufferPool();

    Buffer getBuffer();
    RenoirBufferPoolStats stats() const;
    size_t availableCount() const;
    size_t allocatedCount() const;
    bool canExpand() const;
    size_t shrink(size_t target_available);

    RenoirBufferPoolHandle handle() const { return handle_; }

private:
    friend class SharedMemoryManager;
    friend class SharedMemoryRegion;
    BufferPool(RenoirManagerHandle manager, const std::string& region_name,
               const BufferPoolConfig& config);
    explicit BufferPool(RenoirBufferPoolHandle handle) : handle_(handle) {}

    void destroy();

    RenoirBufferPoolHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for byte ring buffers
 */
class RingBuffer {
public:
    explicit RingBuffer(size_t capacity);
    RingBuffer(const RingBuffer&) = delete;
    RingBuffer& operator=(const RingBuffer&) = delete;

    RingBuffer(RingBuffer&& other) noexcept;
    RingBuffer& operator=(RingBuffer&& other) noexcept;
    ~RingBuffer();

    RenoirRingStats stats() const;
    bool isEmpty() const;
    bool isFull() const;
    size_t availableSpace() const;
    void reset();
    void push(uint8_t data);
    bool tryPush(uint8_t data);
    uint8_t pop();
    bool tryPop(uint8_t* data);
    bool peek(uint8_t* data);

private:
    RenoirRingBufferHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for topic publishers
 */
class Reservation {
public:
    Reservation(const Reservation&) = delete;
    Reservation& operator=(const Reservation&) = delete;
    Reservation(Reservation&& other) noexcept;
    Reservation& operator=(Reservation&& other) noexcept;
    ~Reservation();

    uint8_t* data() const { return data_; }
    size_t capacity() const { return capacity_; }
    RenoirSequenceNumber commit(size_t length);
    void discard();

private:
    friend class Publisher;
    Reservation(RenoirPublisherHandle pub, uint8_t* data, size_t capacity,
                RenoirReservedBufferHandle handle)
        : pub_(pub), data_(data), capacity_(capacity), handle_(handle) {}

    RenoirPublisherHandle pub_ = nullptr;
    uint8_t* data_ = nullptr;
    size_t capacity_ = 0;
    RenoirReservedBufferHandle handle_ = nullptr;
};

enum class ReadResult { Empty, Message, TooSmall };

class Publisher {
public:
    Publisher() = delete;
    Publisher(const Publisher&) = delete;
    Publisher& operator=(const Publisher&) = delete;

    Publisher(Publisher&& other) noexcept;
    Publisher& operator=(Publisher&& other) noexcept;
    ~Publisher();

    void publish(const std::vector<uint8_t>& payload, RenoirSequenceNumber* sequence = nullptr);
    bool tryPublish(const std::vector<uint8_t>& payload,
                    RenoirSequenceNumber* sequence = nullptr);
    Reservation reserve(size_t size);
    bool hasSubscribers() const;
    RenoirTopicStats stats() const;
    std::string topicName() const;

private:
    friend class TopicManager;
    explicit Publisher(RenoirPublisherHandle handle) : handle_(handle) {}

    RenoirPublisherHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for topic subscribers
 */
class Subscriber {
public:
    Subscriber() = delete;
    Subscriber(const Subscriber&) = delete;
    Subscriber& operator=(const Subscriber&) = delete;

    Subscriber(Subscriber&& other) noexcept;
    Subscriber& operator=(Subscriber&& other) noexcept;
    ~Subscriber();

    ReceivedMessage readNext(uint64_t timeout_ms = 1000);
    ReadResult readInto(uint8_t* buffer, size_t capacity, size_t* length,
                        RenoirMessageMetadata* metadata = nullptr);
    bool wait(uint64_t timeout_ms);
    uint64_t skipToLatest(uint64_t keep = 1);
    uint64_t overruns() const;
    bool peek(ReceivedMessage* message);
    size_t drain();
    bool hasMessages() const;
    size_t pendingCount() const;
    RenoirTopicStats stats() const;
    std::string topicName() const;
    int eventFd() const;

private:
    friend class TopicManager;
    explicit Subscriber(RenoirSubscriberHandle handle) : handle_(handle) {}

    RenoirSubscriberHandle handle_ = nullptr;
};

/**
 * @brief RAII wrapper for topic managers
 */
class TopicManager {
public:
    TopicManager();
    TopicManager(const TopicManager&) = delete;
    TopicManager& operator=(const TopicManager&) = delete;

    TopicManager(TopicManager&& other) noexcept;
    TopicManager& operator=(TopicManager&& other) noexcept;
    ~TopicManager();

    RenoirTopicId registerTopic(const std::string& name, const TopicOptions& options);
    RenoirTopicId lookupTopic(const std::string& name) const;
    void removeTopic(const std::string& name);
    size_t topicCount() const;
    std::vector<RenoirTopicInfo> listTopics() const;

    Publisher createPublisher(const std::string& topic_name);
    Subscriber createSubscriber(const std::string& topic_name);
    RenoirTopicStats topicStats(const std::string& topic_name) const;

private:
    RenoirTopicManagerHandle handle_ = nullptr;
};

/**
 * @brief Get library version information
 */
inline std::tuple<uint32_t, uint32_t, uint32_t> getVersion() {
    return {renoir_version_major(), renoir_version_minor(), renoir_version_patch()};
}

/**
 * @brief Get library version string
 */
std::string getVersionString();

} // namespace renoir

#endif // RENOIR_HPP
