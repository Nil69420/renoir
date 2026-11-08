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

  renoir_lean_close(sub);
  renoir_lean_close(pub);
  renoir_lean_close(NULL);
  (void)snprintf(path, sizeof path, "%s/renoir_%s", dir, name);
  (void)unlink(path);
  (void)rmdir(dir);
  puts("lean_abi_test OK");
  return 0;
}
