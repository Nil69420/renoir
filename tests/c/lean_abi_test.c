#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "renoir.h"

#define CHECK(cond)                                                       \
  do {                                                                    \
    if (!(cond)) {                                                        \
      fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #cond);      \
      return 1;                                                           \
    }                                                                     \
  } while (0)

int main(void) {
  RenoirLeanHandle pub = NULL;
  RenoirLeanHandle sub = NULL;
  char dir[] = "/tmp/renoir_lean_XXXXXX";
  char name[64];
  char path[128];
  CHECK(mkdtemp(dir) != NULL);
  CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
  (void)snprintf(name, sizeof name, "lean_abi_c_%d", (int)getpid());

  CHECK(renoir_lean_open(name, 24, 64, 0, &pub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_open(name, 24, 64, RENOIR_LEAN_FROM_OLDEST, &sub) == RenoirErrorCode_Success);
  CHECK(renoir_lean_open(name, 0, 64, 0, &sub) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_open(NULL, 24, 64, 0, &sub) == RenoirErrorCode_InvalidParameter);

  uint8_t msg[24];
  uint8_t out[24];
  CHECK(renoir_lean_try_read(sub, out, sizeof out) == RenoirErrorCode_BufferEmpty);
  for (uint8_t i = 0; i < 10; ++i) {
    memset(msg, i, sizeof msg);
    CHECK(renoir_lean_publish(pub, msg, sizeof msg) == RenoirErrorCode_Success);
  }
  CHECK(renoir_lean_publish(pub, msg, 5) != RenoirErrorCode_Success);
  CHECK(renoir_lean_pending(sub) == 10);
  CHECK(renoir_lean_live_publishers(pub) >= 1);
  for (uint8_t i = 0; i < 10; ++i) {
    CHECK(renoir_lean_try_read(sub, out, sizeof out) == RenoirErrorCode_Success);
    CHECK(out[0] == i && out[23] == i);
  }
  CHECK(renoir_lean_try_read(sub, out, sizeof out) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_try_read(sub, out, 8) != RenoirErrorCode_Success);
  CHECK(renoir_lean_lost(sub) == 0);

  for (uint8_t i = 0; i < 20; ++i) {
    memset(msg, i, sizeof msg);
    CHECK(renoir_lean_publish(pub, msg, sizeof msg) == RenoirErrorCode_Success);
  }
  CHECK(renoir_lean_skip_to_latest(sub, 1) == 19);
  CHECK(renoir_lean_try_read(sub, out, sizeof out) == RenoirErrorCode_Success);
  CHECK(out[0] == 19);

  CHECK(renoir_lean_wait(sub, 0, 0) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_try_read(sub, out, sizeof out) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_wait(sub, 10, 1000000) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_lean_publish_notify(pub, msg, sizeof msg) == RenoirErrorCode_Success);
  CHECK(renoir_lean_wait(sub, 0, 1000000) == RenoirErrorCode_Success);
  CHECK(renoir_lean_wait(NULL, 0, 0) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_wait_adaptive(sub, 1000000) == RenoirErrorCode_Success);
  CHECK(renoir_lean_wait_adaptive(NULL, 0) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_lean_publish_notify(pub, NULL, 24) == RenoirErrorCode_InvalidParameter);

  {
    RenoirLeanHandle b_pub = NULL;
    RenoirLeanHandle b_sub = NULL;
    char b_name[64];
    uint8_t batch[24 * 6];
    uint8_t batch_out[24 * 4];
    size_t n = 0;
    (void)snprintf(b_name, sizeof b_name, "lean_abi_batch_%d", (int)getpid());
    CHECK(renoir_lean_open(b_name, 24, 64, 0, &b_pub) == RenoirErrorCode_Success);
    CHECK(renoir_lean_open(b_name, 24, 64, RENOIR_LEAN_FROM_OLDEST, &b_sub) ==
          RenoirErrorCode_Success);
    for (size_t i = 0; i < 6; ++i) {
      memset(batch + i * 24, (int)(i + 1), 24);
    }
    CHECK(renoir_lean_publish_batch(b_pub, batch, sizeof batch, &n) == RenoirErrorCode_Success);
    CHECK(n == 6);
    CHECK(renoir_lean_publish_batch(b_pub, batch, 25, &n) != RenoirErrorCode_Success);
    CHECK(renoir_lean_publish_batch(b_pub, NULL, 24, &n) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_lean_read_batch(b_sub, batch_out, sizeof batch_out, &n) ==
          RenoirErrorCode_Success);
    CHECK(n == 4 && batch_out[0] == 1 && batch_out[24 * 3] == 4);
    CHECK(renoir_lean_read_batch(b_sub, batch_out, sizeof batch_out, &n) ==
          RenoirErrorCode_Success);
    CHECK(n == 2 && batch_out[24] == 6);
    CHECK(renoir_lean_read_batch(b_sub, batch_out, sizeof batch_out, &n) ==
          RenoirErrorCode_Success);
    CHECK(n == 0);
    renoir_lean_close(b_sub);
    renoir_lean_close(b_pub);
  }

  {
    RenoirLeanHandle mp_a = NULL;
    RenoirLeanHandle mp_b = NULL;
    RenoirLeanHandle wrong = NULL;
    char mp_name[64];
    (void)snprintf(mp_name, sizeof mp_name, "lean_abi_mp_%d", (int)getpid());
    CHECK(renoir_lean_open(mp_name, 24, 64, RENOIR_LEAN_MULTI_PRODUCER, &mp_a) ==
          RenoirErrorCode_Success);
    CHECK(renoir_lean_open(mp_name, 24, 64,
                           RENOIR_LEAN_MULTI_PRODUCER | RENOIR_LEAN_FROM_OLDEST,
                           &mp_b) == RenoirErrorCode_Success);
    CHECK(renoir_lean_open(mp_name, 24, 64, 0, &wrong) != RenoirErrorCode_Success);
    memset(msg, 0xAB, sizeof msg);
    CHECK(renoir_lean_publish(mp_a, msg, sizeof msg) == RenoirErrorCode_Success);
    CHECK(renoir_lean_publish(mp_b, msg, sizeof msg) == RenoirErrorCode_Success);
    CHECK(renoir_lean_pending(mp_b) == 2);
    CHECK(renoir_lean_try_read(mp_b, out, sizeof out) == RenoirErrorCode_Success);
    CHECK(renoir_lean_try_read(mp_b, out, sizeof out) == RenoirErrorCode_Success);
    CHECK(out[0] == 0xAB);
    renoir_lean_close(mp_a);
    renoir_lean_close(mp_b);
  }

  {
    RenoirLeanHandle g_pub = NULL;
    RenoirLeanHandle g_sub = NULL;
    char g_name[64];
    (void)snprintf(g_name, sizeof g_name, "lean_abi_gated_%d", (int)getpid());
    CHECK(renoir_lean_open(g_name, 24, 4, RENOIR_LEAN_GATED, &g_pub) == RenoirErrorCode_Success);
    CHECK(renoir_lean_open(g_name, 24, 4, RENOIR_LEAN_GATED | RENOIR_LEAN_CONSUMER, &g_sub) ==
          RenoirErrorCode_Success);
    memset(msg, 0x5A, sizeof msg);
    for (int i = 0; i < 4; ++i) {
      CHECK(renoir_lean_publish(g_pub, msg, sizeof msg) == RenoirErrorCode_Success);
    }
    CHECK(renoir_lean_publish(g_pub, msg, sizeof msg) == RenoirErrorCode_BufferFull);
    CHECK(renoir_lean_try_read(g_sub, out, sizeof out) == RenoirErrorCode_Success);
    CHECK(renoir_lean_publish(g_pub, msg, sizeof msg) == RenoirErrorCode_Success);
    CHECK(renoir_lean_open(g_name, 24, 4, RENOIR_LEAN_CONSUMER, &g_sub) != RenoirErrorCode_Success);
    renoir_lean_close(g_sub);
    renoir_lean_close(g_pub);
  }

  {
    size_t bytes = 0;
    CHECK(renoir_lean_memory_required(48, 64, &bytes) == RenoirErrorCode_Success);
    CHECK(bytes == 1728 + 64 * 64);
    CHECK(renoir_lean_memory_required(100, 100, &bytes) == RenoirErrorCode_Success);
    CHECK(bytes == 1728 + 64 * 128);
    CHECK(renoir_lean_memory_required(0, 64, &bytes) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_lean_memory_required(505, 64, &bytes) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_lean_memory_required(48, 1, &bytes) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_lean_memory_required(48, 64, NULL) == RenoirErrorCode_InvalidParameter);
  }

  {
    size_t total = 0;
    RenoirRingInfo infos[16];
    int found = 0;
    CHECK(renoir_discover(NULL, 0, &total) == RenoirErrorCode_Success);
    CHECK(total >= 1);
    CHECK(renoir_discover(NULL, 1, &total) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_discover(infos, 16, NULL) == RenoirErrorCode_InvalidParameter);
    CHECK(renoir_discover(infos, 16, &total) == RenoirErrorCode_Success);
    for (size_t i = 0; i < total && i < 16; ++i) {
      if (strcmp(infos[i].name, name) == 0) {
        found = 1;
        CHECK(infos[i].kind == RENOIR_RING_KIND_LEAN);
        CHECK(infos[i].msg_size == 24);
        CHECK(infos[i].slot_count == 64);
        CHECK(infos[i].live_publishers >= 1);
        CHECK(infos[i].mapped_by_count >= 1);
      }
    }
    CHECK(found == 1);
    size_t removed = 99;
    char mine[160];
    (void)snprintf(mine, sizeof mine, "%s/renoir_%s", dir, name);
    CHECK(renoir_discover_sweep(RENOIR_SWEEP_DRY_RUN | RENOIR_SWEEP_IGNORE_UNREADABLE, &removed) ==
          RenoirErrorCode_Success);
    CHECK(access(mine, F_OK) == 0);
    size_t dry_removed = removed;
    CHECK(renoir_discover_sweep(RENOIR_SWEEP_IGNORE_UNREADABLE, &removed) ==
          RenoirErrorCode_Success);
    CHECK(removed == dry_removed);
    CHECK(access(mine, F_OK) == 0);
    CHECK(renoir_discover(infos, 16, &total) == RenoirErrorCode_Success);
    CHECK(total == 1);
  }

  renoir_lean_close(sub);
  renoir_lean_close(pub);
  renoir_lean_close(NULL);
  (void)snprintf(path, sizeof path, "%s/renoir_%s", dir, name);
  (void)unlink(path);
  (void)rmdir(dir);
  puts("lean_abi_test OK");
  return 0;
}
