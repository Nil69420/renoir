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
  (void)snprintf(out, n, "c_qos_%s_%d", tag, (int)getpid());
}

static int defaults_and_validation(void) {
  RenoirLeanQos q;
  char name[64];
  RenoirLeanHandle h = NULL;
  name_for(name, sizeof name, "valid");
  CHECK(renoir_lean_qos_default(NULL) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_qos_default(&q) == RenoirErrorCode_Success);
  CHECK(q.depth == 64 && q.reliability == RENOIR_QOS_BEST_EFFORT && q.ownership == RENOIR_QOS_OWNERSHIP_SHARED);
  CHECK(renoir_lean_open_qos(NULL, 16, &q, 0, &h) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_open_qos(name, 16, NULL, 0, &h) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, NULL) == RenoirErrorCode_InvalidParameter);
  q.reliability = 7;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &h) == RenoirErrorCode_InvalidParameter);
  q.reliability = RENOIR_QOS_BEST_EFFORT;
  q.history = RENOIR_QOS_KEEP_ALL;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &h) == RenoirErrorCode_InvalidParameter);
  q.history = RENOIR_QOS_KEEP_LAST;
  q.lifespan_ns = 5 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &h) == RenoirErrorCode_InvalidParameter);
  q.lifespan_ns = 0;
  q.depth = 1;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &h) == RenoirErrorCode_InvalidParameter);
  q.depth = 16;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &h) == RenoirErrorCode_Success);
  renoir_lean_close(h);
  return 0;
}

static int compatibility_and_liveliness(void) {
  RenoirLeanQos offer;
  RenoirLeanQos want;
  RenoirLeanHandle pub = NULL;
  RenoirLeanHandle sub = NULL;
  RenoirLeanHandle bad = NULL;
  RenoirLivelinessStatus st;
  char name[64];
  name_for(name, sizeof name, "live");
  (void)renoir_lean_qos_default(&offer);
  offer.liveliness_kind = RENOIR_QOS_LIVELINESS_MANUAL;
  offer.liveliness_lease_ns = 100 * MS;
  offer.deadline_ns = 10 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &offer, 0, &pub) == RenoirErrorCode_Success);

  want = offer;
  want.deadline_ns = 5 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &want, RENOIR_LEAN_CONSUMER, &bad) == RenoirErrorCode_InvalidParameter);
  want = offer;
  want.liveliness_lease_ns = 50 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &want, RENOIR_LEAN_CONSUMER, &bad) == RenoirErrorCode_InvalidParameter);
  want = offer;
  want.liveliness_lease_ns = 200 * MS;
  want.deadline_ns = 20 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &want, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);

  CHECK(renoir_lean_liveliness(sub, NULL) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_liveliness(NULL, &st) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success && st.lost == 1);
  CHECK(renoir_lean_register_publisher(pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success && st.alive == 1 && st.lost == 0);
  sleep_ms(300);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success);
  CHECK(st.lost == 1 && st.registered == 1 && st.lost_count == 1);
  CHECK(renoir_lean_assert_liveliness(pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success && st.alive == 1);
  renoir_lean_close(sub);
  renoir_lean_close(pub);
  return 0;
}

static int ownership(void) {
  RenoirLeanQos q;
  RenoirLeanHandle weak = NULL;
  RenoirLeanHandle strong = NULL;
  uint8_t buf[16] = {0};
  char name[64];
  name_for(name, sizeof name, "own");
  (void)renoir_lean_qos_default(&q);
  q.ownership = RENOIR_QOS_OWNERSHIP_EXCLUSIVE;
  q.ownership_strength = 5;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &weak) == RenoirErrorCode_Success);
  q.ownership_strength = 9;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &strong) == RenoirErrorCode_Success);
  CHECK(renoir_lean_register_publisher(weak) == RenoirErrorCode_Success);
  CHECK(renoir_lean_register_publisher(strong) == RenoirErrorCode_Success);
  CHECK(renoir_lean_publish(weak, buf, sizeof buf) == RenoirErrorCode_ConcurrencyError);
  CHECK(renoir_lean_publish(strong, buf, sizeof buf) == RenoirErrorCode_Success);
  CHECK(renoir_lean_is_owner(strong) == 1 && renoir_lean_is_owner(weak) == 0);
  CHECK(renoir_lean_is_owner(NULL) == -1);
  renoir_lean_close(strong);
  sleep_ms(10);
  CHECK(renoir_lean_publish(weak, buf, sizeof buf) == RenoirErrorCode_Success);
  renoir_lean_close(weak);
  return 0;
}

static int lifespan_and_filter(void) {
  RenoirLeanQos q;
  RenoirLeanHandle pub = NULL;
  RenoirLeanHandle sub = NULL;
  uint8_t buf[16] = {0};
  uint64_t stamp = 0;
  char name[64];
  name_for(name, sizeof name, "life");
  (void)renoir_lean_qos_default(&q);
  q.clock = RENOIR_LEAN_CLOCK_NS;
  q.lifespan_ns = 30 * MS;
  q.min_separation_ns = 0;
  q.durability = RENOIR_QOS_TRANSIENT_LOCAL;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &pub) == RenoirErrorCode_Success);
  buf[0] = 1;
  CHECK(renoir_lean_publish_now(pub, buf, sizeof buf) == RenoirErrorCode_Success);
  sleep_ms(80);
  buf[0] = 2;
  CHECK(renoir_lean_publish_now(pub, buf, sizeof buf) == RenoirErrorCode_Success);
  CHECK(renoir_lean_open_qos(name, 16, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_try_read_stamped_qos(sub, buf, sizeof buf, &stamp) == RenoirErrorCode_Success && buf[0] == 2);
  CHECK(renoir_lean_stale_dropped(sub) == 1 && renoir_lean_filtered_out(sub) == 0);
  CHECK(renoir_lean_try_read_stamped_qos(sub, buf, sizeof buf, &stamp) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_try_read_stamped_qos(sub, buf, sizeof buf, NULL) == RenoirErrorCode_InvalidParameter);
  renoir_lean_close(sub);
  renoir_lean_close(pub);

  (void)snprintf(name, sizeof name, "c_qos_filter_%d", (int)getpid());
  q.lifespan_ns = 0;
  q.min_separation_ns = 5 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &pub) == RenoirErrorCode_Success);
  for (uint64_t i = 0; i < 21; ++i) {
    CHECK(renoir_lean_publish_at(pub, i * MS, buf, sizeof buf) == RenoirErrorCode_Success);
  }
  CHECK(renoir_lean_open_qos(name, 16, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  int delivered = 0;
  while (renoir_lean_try_read_stamped_qos(sub, buf, sizeof buf, &stamp) == RenoirErrorCode_Success) {
    delivered += 1;
  }
  CHECK(delivered == 5 && renoir_lean_filtered_out(sub) == 16);
  renoir_lean_close(sub);
  renoir_lean_close(pub);
  return 0;
}

static int strict_and_wait(void) {
  RenoirLeanQos q;
  RenoirLeanHandle plain = NULL;
  RenoirLeanHandle sub = NULL;
  RenoirLeanHandle pub = NULL;
  RenoirLivelinessStatus st;
  uint8_t buf[16] = {0};
  char name[64];
  name_for(name, sizeof name, "strict");
  CHECK(renoir_lean_open(name, 16, 8, 0, &plain) == RenoirErrorCode_Success);
  (void)renoir_lean_qos_default(&q);
  q.depth = 8;
  q.flags = RENOIR_QOS_FLAG_REQUIRE_OFFER;
  CHECK(renoir_lean_open_qos(name, 16, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_InvalidParameter);
  q.flags = 0;
  CHECK(renoir_lean_open_qos(name, 16, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  renoir_lean_close(sub);
  renoir_lean_close(plain);

  name_for(name, sizeof name, "wait");
  (void)renoir_lean_qos_default(&q);
  q.liveliness_kind = RENOIR_QOS_LIVELINESS_MANUAL;
  q.liveliness_lease_ns = 80 * MS;
  CHECK(renoir_lean_open_qos(name, 16, &q, 0, &pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_open_qos(name, 16, &q, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_wait_liveliness_change(sub, 20 * MS, &st) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_register_publisher(pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success && st.alive == 1);
  CHECK(renoir_lean_wait_liveliness_change(sub, 3000 * MS, &st) == RenoirErrorCode_Success);
  CHECK(st.lost == 1);
  CHECK(renoir_lean_publish(pub, buf, sizeof buf) == RenoirErrorCode_Success);
  CHECK(renoir_lean_liveliness(sub, &st) == RenoirErrorCode_Success && st.lost == 0);
  CHECK(renoir_lean_wait_liveliness_change(NULL, 1, &st) == RenoirErrorCode_InvalidParameter);
  renoir_lean_close(sub);
  renoir_lean_close(pub);
  return 0;
}

int main(void) {
  CHECK(defaults_and_validation() == 0);
  CHECK(compatibility_and_liveliness() == 0);
  CHECK(ownership() == 0);
  CHECK(lifespan_and_filter() == 0);
  CHECK(strict_and_wait() == 0);
  puts("lean qos ABI ok");
  return 0;
}
