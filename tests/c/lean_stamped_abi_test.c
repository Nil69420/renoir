#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "renoir.h"

#define CHECK(cond)                                                  \
  do {                                                               \
    if (!(cond)) {                                                   \
      fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #cond); \
      return 1;                                                      \
    }                                                                \
  } while (0)

struct sample {
  uint32_t id;
  float v[3];
};

static int run(uint32_t stamp_flag, uint32_t clock_kind, const char *tag) {
  char name[64];
  (void)snprintf(name, sizeof name, "c_stamped_%s_%d", tag, (int)getpid());
  RenoirLeanHandle pub = NULL;
  RenoirLeanHandle sub = NULL;
  CHECK(renoir_lean_open(name, sizeof(struct sample), 64, stamp_flag, &pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_open(name, sizeof(struct sample), 64, stamp_flag | RENOIR_LEAN_FROM_OLDEST, &sub) ==
        RenoirErrorCode_Success);
  CHECK(renoir_lean_clock_kind(pub) == clock_kind);
  if (clock_kind == RENOIR_LEAN_CLOCK_TICKS) {
    CHECK(renoir_lean_tick_hz(pub) > 1000000U);
    CHECK(renoir_lean_stamp_to_ns(pub, renoir_lean_tick_hz(pub)) == 1000000000ULL);
  }

  struct sample in = {7, {1.0F, 2.0F, 3.0F}};
  struct sample out;
  uint64_t stamp = 0;
  CHECK(renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out, &stamp) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_publish_at(pub, 424242, (const uint8_t *)&in, sizeof in) == RenoirErrorCode_Success);
  CHECK(renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out, &stamp) == RenoirErrorCode_Success);
  CHECK(stamp == 424242 && memcmp(&in, &out, sizeof in) == 0);

  uint64_t before = renoir_lean_stamp_now(pub);
  CHECK(renoir_lean_publish_now(pub, (const uint8_t *)&in, sizeof in) == RenoirErrorCode_Success);
  uint64_t after = renoir_lean_stamp_now(pub);
  CHECK(renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out, &stamp) == RenoirErrorCode_Success);
  CHECK(stamp >= before && stamp <= after);

  CHECK(renoir_lean_publish(pub, (const uint8_t *)&in, sizeof in) != RenoirErrorCode_Success);
  CHECK(renoir_lean_publish_at(pub, 1, (const uint8_t *)&in, sizeof in - 1) != RenoirErrorCode_Success);
  CHECK(renoir_lean_publish_at(pub, 1, NULL, sizeof in) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out, NULL) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out - 1, &stamp) != RenoirErrorCode_Success);

  renoir_lean_close(sub);
  renoir_lean_close(pub);
  return 0;
}

int main(void) {
  size_t plain_bytes = 0;
  size_t stamped_bytes = 0;
  CHECK(renoir_lean_memory_required_flags(56, 64, 0, &plain_bytes) == RenoirErrorCode_Success);
  CHECK(renoir_lean_memory_required_flags(56, 64, RENOIR_LEAN_STAMP_NS, &stamped_bytes) == RenoirErrorCode_Success);
  CHECK(stamped_bytes > plain_bytes);
  CHECK(renoir_lean_memory_required_flags(UINT32_MAX, 64, RENOIR_LEAN_STAMP_NS, &stamped_bytes) ==
        RenoirErrorCode_InvalidParameter);
  RenoirLeanHandle bad = NULL;
  CHECK(renoir_lean_open("c_stamped_bad", 16, 64, RENOIR_LEAN_STAMP_NS | RENOIR_LEAN_STAMP_TICKS, &bad) ==
        RenoirErrorCode_InvalidParameter);
  CHECK(run(RENOIR_LEAN_STAMP_NS, RENOIR_LEAN_CLOCK_NS, "ns") == 0);
  CHECK(run(RENOIR_LEAN_STAMP_TICKS, RENOIR_LEAN_CLOCK_TICKS, "ticks") == 0);

  RenoirLeanHandle plain = NULL;
  char name[64];
  uint64_t stamp = 0;
  uint8_t buf[16];
  (void)snprintf(name, sizeof name, "c_unstamped_%d", (int)getpid());
  CHECK(renoir_lean_open(name, 16, 64, 0, &plain) == RenoirErrorCode_Success);
  CHECK(renoir_lean_clock_kind(plain) == RENOIR_LEAN_CLOCK_NONE);
  CHECK(renoir_lean_publish_at(plain, 1, buf, sizeof buf) != RenoirErrorCode_Success);
  CHECK(renoir_lean_try_read_stamped(plain, buf, sizeof buf, &stamp) != RenoirErrorCode_Success);
  renoir_lean_close(plain);
  puts("lean stamped ABI ok");
  return 0;
}
