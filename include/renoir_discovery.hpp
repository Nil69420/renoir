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
    const RenoirRingInfo& r = buffer[i];
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

}

#endif
