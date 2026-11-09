#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "renoir.h"

#define CHECK(cond)                                                  \
  do {                                                               \
    if (!(cond)) {                                                   \
      fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #cond); \
      return 1;                                                      \
    }                                                                \
  } while (0)

#define MS 1000000ULL

static void sleep_ms(long ms) {
  struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
  (void)nanosleep(&ts, NULL);
}

static void name_for(char *out, size_t n, const char *tag) {
  (void)snprintf(out, n, "c_fqos_%s_%d", tag, (int)getpid());
}

static int commit_byte(RenoirFrameHandle h, uint8_t value) {
  uint8_t *dst = NULL;
  CHECK(renoir_frame_loan(h, 64, &dst) == RenoirErrorCode_Success);
  memset(dst, value, 64);
  CHECK(renoir_frame_commit(h) == RenoirErrorCode_Success);
  return 0;
}

int main(void) {
  RenoirLeanQos q;
  RenoirFrameHandle pub = NULL;
  RenoirFrameHandle sub = NULL;
  RenoirLivelinessStatus st;
  const uint8_t *data = NULL;
  size_t len = 0;
  char name[64];

  name_for(name, sizeof name, "valid");
  (void)renoir_lean_qos_default(&q);
  CHECK(renoir_frame_open_qos(name, 64, &q, 0, &pub) == RenoirErrorCode_InvalidParameter);
  q.reliability = RENOIR_QOS_RELIABLE;
  q.depth = 8;
  q.clock = RENOIR_LEAN_CLOCK_NS;
  CHECK(renoir_frame_open_qos(name, 64, &q, 0, &pub) == RenoirErrorCode_InvalidParameter);
  q.clock = 0;
  CHECK(renoir_frame_open_qos(NULL, 64, &q, 0, &pub) == RenoirErrorCode_InvalidParameter);

  name_for(name, sizeof name, "life");
  q.lifespan_ns = 40 * MS;
  q.durability = RENOIR_QOS_TRANSIENT_LOCAL;
  q.liveliness_kind = RENOIR_QOS_LIVELINESS_MANUAL;
  q.liveliness_lease_ns = 100 * MS;
  q.deadline_ns = 10 * MS;
  CHECK(renoir_frame_open_qos(name, 64, &q, 0, &pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_open_qos(name, 64, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_wait_deadline(sub, 10) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_frame_deadline_misses(sub) == 1);
  CHECK(renoir_frame_liveliness(sub, &st) == RenoirErrorCode_Success && st.lost == 1);
  CHECK(renoir_frame_register_publisher(pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_liveliness(sub, &st) == RenoirErrorCode_Success && st.alive == 1);
  CHECK(commit_byte(pub, 1) == 0);
  sleep_ms(120);
  CHECK(commit_byte(pub, 2) == 0);
  CHECK(renoir_frame_view_qos(sub, &data, &len) == RenoirErrorCode_Success && len == 64 && data[0] == 2);
  CHECK(renoir_frame_release(sub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_stale_dropped(sub) == 1);
  CHECK(renoir_frame_wait_liveliness_change(sub, 3000 * MS, &st) == RenoirErrorCode_Success);
  CHECK(st.lost == 1);
  CHECK(renoir_frame_assert_liveliness(pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_liveliness(sub, &st) == RenoirErrorCode_Success && st.lost == 0);
  CHECK(renoir_frame_wait_liveliness_change(NULL, 1, &st) == RenoirErrorCode_InvalidParameter);
  renoir_frame_close(sub);
  renoir_frame_close(pub);

  RenoirFrameHandle weak = NULL;
  RenoirFrameHandle strong = NULL;
  name_for(name, sizeof name, "own");
  (void)renoir_lean_qos_default(&q);
  q.reliability = RENOIR_QOS_RELIABLE;
  q.depth = 8;
  q.ownership = RENOIR_QOS_OWNERSHIP_EXCLUSIVE;
  q.ownership_strength = 3;
  CHECK(renoir_frame_open_qos(name, 64, &q, 0, &weak) == RenoirErrorCode_Success);
  q.ownership_strength = 8;
  CHECK(renoir_frame_open_qos(name, 64, &q, 0, &strong) == RenoirErrorCode_Success);
  CHECK(renoir_frame_register_publisher(weak) == RenoirErrorCode_Success);
  CHECK(renoir_frame_register_publisher(strong) == RenoirErrorCode_Success);
  uint8_t *dst = NULL;
  CHECK(renoir_frame_loan(weak, 64, &dst) == RenoirErrorCode_ConcurrencyError);
  CHECK(commit_byte(strong, 9) == 0);
  CHECK(renoir_frame_is_owner(strong) == 1 && renoir_frame_is_owner(weak) == 0);
  CHECK(renoir_frame_is_owner(NULL) == -1);
  renoir_frame_close(strong);
  sleep_ms(10);
  CHECK(commit_byte(weak, 7) == 0);
  renoir_frame_close(weak);
  puts("frame qos ABI ok");
  return 0;
}
