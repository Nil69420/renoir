#ifndef RENOIR_DISCOVERY_HPP
#define RENOIR_DISCOVERY_HPP

#include <cstddef>
#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include "renoir.h"

namespace renoir {

enum class RingKind : uint32_t {
  Lean = RENOIR_RING_KIND_LEAN,
  Broadcast = RENOIR_RING_KIND_BROADCAST,
  Spsc = RENOIR_RING_KIND_SPSC,
  Mpmc = RENOIR_RING_KIND_MPMC
};

struct RingInfo {
  std::string name;
  RingKind kind;
  bool hasMode;
  uint32_t mode;
  uint32_t msgSize;
  uint32_t slotCount;
  uint64_t published;
  uint64_t fileBytes;
  uint32_t livePublishers;
  uint32_t registeredConsumers;
  uint32_t mappedByCount;
};

inline RingInfo toRingInfo(const RenoirRingInfo& r) {
  RingInfo info;
  info.name = std::string(r.name);
  info.kind = static_cast<RingKind>(r.kind);
  info.hasMode = r.mode != RENOIR_RING_MODE_NONE;
  info.mode = r.mode;
  info.msgSize = r.msg_size;
  info.slotCount = r.slot_count;
  info.published = r.published;
  info.fileBytes = r.file_bytes;
  info.livePublishers = r.live_publishers;
  info.registeredConsumers = r.registered_consumers;
  info.mappedByCount = r.mapped_by_count;
  return info;
}

inline std::vector<RingInfo> discover() {
  std::vector<RingInfo> result;
  std::vector<RenoirRingInfo> buffer;
  size_t total = 0;
  if (renoir_discover(nullptr, 0, &total) != RenoirErrorCode_Success) {
    return result;
  }
  bool complete = false;
  for (int attempt = 0; attempt < 3 && !complete; ++attempt) {
    buffer.resize(total + 8);
    size_t filled = 0;
    if (renoir_discover(buffer.data(), buffer.size(), &filled) != RenoirErrorCode_Success) {
      return result;
    }
    complete = filled <= buffer.size();
    total = filled;
  }
  if (!complete) {
    return result;
  }
  result.reserve(total);
  for (size_t i = 0; i < total; ++i) {
    RingInfo info = toRingInfo(buffer[i]);
    result.push_back(std::move(info));
  }
  return result;
}

inline size_t sweep(bool dryRun = false, bool ignoreUnreadable = false) noexcept {
  uint32_t flags = 0;
  if (dryRun) {
    flags |= RENOIR_SWEEP_DRY_RUN;
  }
  if (ignoreUnreadable) {
    flags |= RENOIR_SWEEP_IGNORE_UNREADABLE;
  }
  size_t removed = 0;
  if (renoir_discover_sweep(flags, &removed) != RenoirErrorCode_Success) {
    return 0;
  }
  return removed;
}

enum class DiscoveryEventKind : uint32_t {
  Appeared = RENOIR_DISCOVERY_APPEARED,
  Disappeared = RENOIR_DISCOVERY_DISAPPEARED,
  Changed = RENOIR_DISCOVERY_CHANGED
};

struct DiscoveryEvent {
  DiscoveryEventKind kind;
  RingInfo ring;
  uint32_t previousLivePublishers;
  uint32_t previousRegisteredConsumers;
};

class DiscoveryWatch {
 public:
  explicit DiscoveryWatch(uint32_t intervalMs = 100) {
    if (renoir_discover_watch_open(intervalMs, &handle_) != RenoirErrorCode_Success) {
      handle_ = nullptr;
    }
  }

  DiscoveryWatch(const DiscoveryWatch&) = delete;
  DiscoveryWatch& operator=(const DiscoveryWatch&) = delete;

  DiscoveryWatch(DiscoveryWatch&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
  }

  DiscoveryWatch& operator=(DiscoveryWatch&& other) noexcept {
    if (this != &other) {
      reset();
      handle_ = other.handle_;
      other.handle_ = nullptr;
    }
    return *this;
  }

  ~DiscoveryWatch() { reset(); }

  bool valid() const noexcept { return handle_ != nullptr; }
  int fd() const noexcept { return renoir_discover_watch_fd(handle_); }

  std::vector<DiscoveryEvent> poll(uint32_t timeoutMs, size_t capacity = 64) {
    std::vector<DiscoveryEvent> result;
    std::vector<RenoirDiscoveryEvent> buffer(capacity);
    size_t count = 0;
    if (renoir_discover_watch_poll(handle_, timeoutMs, buffer.data(), buffer.size(), &count) !=
        RenoirErrorCode_Success) {
      return result;
    }
    result.reserve(count);
    for (size_t i = 0; i < count; ++i) {
      const RenoirDiscoveryEvent& e = buffer[i];
      result.push_back(DiscoveryEvent{static_cast<DiscoveryEventKind>(e.kind), toRingInfo(e.ring),
                                      e.previous_live_publishers,
                                      e.previous_registered_consumers});
    }
    return result;
  }

 private:
  void reset() noexcept {
    if (handle_ != nullptr) {
      renoir_discover_watch_close(handle_);
      handle_ = nullptr;
    }
  }

  RenoirDiscoveryWatchHandle handle_ = nullptr;
};

}

#endif
