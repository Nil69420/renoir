#ifndef RENOIR_FRAME_HPP
#define RENOIR_FRAME_HPP

#include "renoir.hpp"

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <utility>

namespace renoir {

enum class FrameFlags : uint32_t {
    None = 0,
    FromOldest = RENOIR_LEAN_FROM_OLDEST,
    MultiProducer = RENOIR_LEAN_MULTI_PRODUCER,
    Consumer = RENOIR_LEAN_CONSUMER,
};

constexpr FrameFlags operator|(FrameFlags a, FrameFlags b) noexcept {
    return static_cast<FrameFlags>(static_cast<uint32_t>(a) | static_cast<uint32_t>(b));
}

class FrameChannel {
public:
    static constexpr size_t kMaxFrameBytes = size_t{64} << 20;

    class Loan {
    public:
        Loan(const Loan&) = delete;
        Loan& operator=(const Loan&) = delete;
        Loan(Loan&& other) noexcept
            : channel_(other.channel_), data_(other.data_), size_(other.size_) {
            other.channel_ = nullptr;
        }
        Loan& operator=(Loan&& other) noexcept {
            if (this != &other) {
                abort();
                channel_ = other.channel_;
                data_ = other.data_;
                size_ = other.size_;
                other.channel_ = nullptr;
            }
            return *this;
        }
        ~Loan() { abort(); }

        uint8_t* data() noexcept { return data_; }
        size_t size() const noexcept { return size_; }

        void commit(bool notify = false) {
            if (channel_ == nullptr) {
                return;
            }
            const RenoirFrameHandle h = channel_->handle_;
            channel_ = nullptr;
            checkResult(notify ? renoir_frame_commit_notify(h) : renoir_frame_commit(h),
                        "frame commit");
        }

        void abort() noexcept {
            if (channel_ != nullptr) {
                renoir_frame_abort(channel_->handle_);
                channel_ = nullptr;
            }
        }

    private:
        friend class FrameChannel;
        Loan(FrameChannel* channel, uint8_t* data, size_t size) noexcept
            : channel_(channel), data_(data), size_(size) {}
        FrameChannel* channel_;
        uint8_t* data_;
        size_t size_;
    };

    class View {
    public:
        View(const View&) = delete;
        View& operator=(const View&) = delete;
        View(View&& other) noexcept
            : channel_(other.channel_), data_(other.data_), size_(other.size_) {
            other.channel_ = nullptr;
        }
        View& operator=(View&& other) noexcept {
            if (this != &other) {
                release();
                channel_ = other.channel_;
                data_ = other.data_;
                size_ = other.size_;
                other.channel_ = nullptr;
            }
            return *this;
        }
        ~View() { release(); }

        const uint8_t* data() const noexcept { return data_; }
        size_t size() const noexcept { return size_; }
        bool intact() const noexcept {
            return channel_ != nullptr && renoir_frame_view_intact(channel_->handle_);
        }

        void release() noexcept {
            if (channel_ != nullptr) {
                renoir_frame_release(channel_->handle_);
                channel_ = nullptr;
            }
        }

    private:
        friend class FrameChannel;
        View(FrameChannel* channel, const uint8_t* data, size_t size) noexcept
            : channel_(channel), data_(data), size_(size) {}
        FrameChannel* channel_;
        const uint8_t* data_;
        size_t size_;
    };

    FrameChannel(const std::string& name, size_t maxFrame, uint32_t slotCount, FrameFlags flags) {
        checkResult(renoir_frame_open(name.c_str(), maxFrame, slotCount,
                                      static_cast<uint32_t>(flags), &handle_),
                    "frame open");
    }

    FrameChannel(const FrameChannel&) = delete;
    FrameChannel& operator=(const FrameChannel&) = delete;
    FrameChannel(FrameChannel&&) = delete;
    FrameChannel& operator=(FrameChannel&&) = delete;

    ~FrameChannel() { renoir_frame_close(handle_); }

    std::optional<Loan> tryLoan(size_t len) {
        uint8_t* data = nullptr;
        const RenoirErrorCode rc = renoir_frame_loan(handle_, len, &data);
        if (rc == RenoirErrorCode::RenoirErrorCode_BufferFull) {
            return std::nullopt;
        }
        checkResult(rc, "frame loan");
        return Loan(this, data, len);
    }

    std::optional<View> tryView() {
        const uint8_t* data = nullptr;
        size_t len = 0;
        const RenoirErrorCode rc = renoir_frame_view(handle_, &data, &len);
        if (rc == RenoirErrorCode::RenoirErrorCode_BufferEmpty) {
            return std::nullopt;
        }
        checkResult(rc, "frame view");
        return View(this, data, len);
    }

    std::optional<View> tryViewFresh(uint64_t maxAgeNs) {
        const uint8_t* data = nullptr;
        size_t len = 0;
        const RenoirErrorCode rc = renoir_frame_view_fresh(handle_, maxAgeNs, &data, &len);
        if (rc == RenoirErrorCode::RenoirErrorCode_BufferEmpty) {
            return std::nullopt;
        }
        checkResult(rc, "frame view fresh");
        return View(this, data, len);
    }

    uint64_t staleDropped() const noexcept { return renoir_frame_stale_dropped(handle_); }

    bool wait(uint32_t spinIterations = 2000, uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_frame_wait(handle_, spinIterations, timeoutNs) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    int consumerId() const noexcept { return renoir_frame_consumer_id(handle_); }

    void setUpstream(uint32_t upstream) {
        checkResult(renoir_frame_set_upstream(handle_, upstream), "frame set upstream");
    }

    size_t livePublishers() const noexcept { return renoir_frame_live_publishers(handle_); }
    size_t maxFrame() const noexcept { return renoir_frame_max_frame(handle_); }

private:
    RenoirFrameHandle handle_ = nullptr;
};

}

#endif
