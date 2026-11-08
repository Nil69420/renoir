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
  CHECK(renoir_lean_publish_notify(pub, NULL, 24) == RenoirErrorCode_InvalidParameter);

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

  renoir_lean_close(sub);
  renoir_lean_close(pub);
  renoir_lean_close(NULL);
  (void)snprintf(path, sizeof path, "%s/renoir_%s", dir, name);
  (void)unlink(path);
  (void)rmdir(dir);
  puts("lean_abi_test OK");
  return 0;
}
