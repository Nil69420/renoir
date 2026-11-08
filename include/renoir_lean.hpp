#ifndef RENOIR_LEAN_HPP
#define RENOIR_LEAN_HPP

#include "renoir.hpp"

#include <cstddef>
#include <cstdint>
#include <string>
#include <type_traits>

namespace renoir {

enum class LeanFlags : uint32_t {
    None = 0,
    FromOldest = RENOIR_LEAN_FROM_OLDEST,
    MultiProducer = RENOIR_LEAN_MULTI_PRODUCER,
    Gated = RENOIR_LEAN_GATED,
    Consumer = RENOIR_LEAN_CONSUMER,
    ReliablePublisher = RENOIR_LEAN_RELIABLE_PUBLISHER,
    ReliableSubscriber = RENOIR_LEAN_RELIABLE_SUBSCRIBER,
};

constexpr LeanFlags operator|(LeanFlags a, LeanFlags b) noexcept {
    return static_cast<LeanFlags>(static_cast<uint32_t>(a) | static_cast<uint32_t>(b));
}

class LeanChannel {
public:
    static constexpr size_t kMaxMessageBytes = 504;

    LeanChannel(const std::string& name, uint32_t messageSize, uint32_t slotCount,
                bool fromOldest = false, bool multiProducer = false) {
        const uint32_t flags = (fromOldest ? RENOIR_LEAN_FROM_OLDEST : 0U) |
                               (multiProducer ? RENOIR_LEAN_MULTI_PRODUCER : 0U);
        checkResult(renoir_lean_open(name.c_str(), messageSize, slotCount, flags, &handle_),
                    "lean open");
        messageSize_ = messageSize;
    }

    LeanChannel(const std::string& name, uint32_t messageSize, uint32_t slotCount, LeanFlags flags) {
        checkResult(renoir_lean_open(name.c_str(), messageSize, slotCount,
                                     static_cast<uint32_t>(flags), &handle_),
                    "lean open");
        messageSize_ = messageSize;
    }

    LeanChannel(const LeanChannel&) = delete;
    LeanChannel& operator=(const LeanChannel&) = delete;

    LeanChannel(LeanChannel&& other) noexcept
        : handle_(other.handle_), messageSize_(other.messageSize_) {
        other.handle_ = nullptr;
    }

    LeanChannel& operator=(LeanChannel&& other) noexcept {
        if (this != &other) {
            renoir_lean_close(handle_);
            handle_ = other.handle_;
            messageSize_ = other.messageSize_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~LeanChannel() { renoir_lean_close(handle_); }

    RenoirErrorCode publish(const void* data, size_t len) noexcept {
        return renoir_lean_publish(handle_, static_cast<const uint8_t*>(data), len);
    }

    RenoirErrorCode publishNotify(const void* data, size_t len) noexcept {
        return renoir_lean_publish_notify(handle_, static_cast<const uint8_t*>(data), len);
    }

    RenoirErrorCode publishBatch(const void* data, size_t len, size_t& published) noexcept {
        return renoir_lean_publish_batch(handle_, static_cast<const uint8_t*>(data), len,
                                         &published);
    }

    RenoirErrorCode readBatch(void* out, size_t len, size_t& count) noexcept {
        return renoir_lean_read_batch(handle_, static_cast<uint8_t*>(out), len, &count);
    }

    void setDeadline(uint64_t deadlineNs) noexcept { renoir_lean_set_deadline(handle_, deadlineNs); }

    bool waitDeadline(uint32_t spinIterations = 2000) noexcept {
        return renoir_lean_wait_deadline(handle_, spinIterations) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    uint64_t deadlineMisses() const noexcept { return renoir_lean_deadline_misses(handle_); }

    bool waitAdaptive(uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_lean_wait_adaptive(handle_, timeoutNs) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    bool wait(uint32_t spinIterations = 2000, uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_lean_wait(handle_, spinIterations, timeoutNs) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    template <typename T>
    RenoirErrorCode publish(const T& value) noexcept {
        static_assert(std::is_trivially_copyable<T>::value, "T must be trivially copyable");
        static_assert(sizeof(T) <= kMaxMessageBytes, "T exceeds the lean message limit");
        return publish(&value, sizeof(T));
    }

    bool tryRead(void* out, size_t len) noexcept {
        return renoir_lean_try_read(handle_, static_cast<uint8_t*>(out), len) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    template <typename T>
    bool tryRead(T& value) noexcept {
        static_assert(std::is_trivially_copyable<T>::value, "T must be trivially copyable");
        static_assert(sizeof(T) <= kMaxMessageBytes, "T exceeds the lean message limit");
        return tryRead(&value, sizeof(T));
    }

    uint64_t skipToLatest(uint64_t keep = 1) noexcept {
        return renoir_lean_skip_to_latest(handle_, keep);
    }

    uint64_t lost() const noexcept { return renoir_lean_lost(handle_); }
    size_t pending() const noexcept { return renoir_lean_pending(handle_); }
    uint32_t messageSize() const noexcept { return messageSize_; }

private:
    RenoirLeanHandle handle_ = nullptr;
    uint32_t messageSize_ = 0;
};

}  // namespace renoir

#endif
