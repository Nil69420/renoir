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
  RenoirFrameHandle pub = NULL;
  RenoirFrameHandle sub = NULL;
  char dir[] = "/tmp/renoir_frame_XXXXXX";
  char name[64];
  char path[128];
  uint8_t *out = NULL;
  const uint8_t *in = NULL;
  size_t len = 0;
  CHECK(mkdtemp(dir) != NULL);
  CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
  (void)snprintf(name, sizeof name, "frame_abi_%d", (int)getpid());

  CHECK(renoir_frame_open(NULL, 1000, 4, 0, &pub) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_frame_open(name, 0, 4, 0, &pub) != RenoirErrorCode_Success);
  CHECK(renoir_frame_open(name, 4096, 4, RENOIR_LEAN_CONSUMER, &sub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_open(name, 4096, 4, 0, &pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_max_frame(pub) == 4096);
  CHECK(renoir_frame_view(pub, &in, &len) != RenoirErrorCode_Success);
  CHECK(renoir_frame_view(sub, &in, &len) == RenoirErrorCode_BufferEmpty);

  const size_t sizes[4] = {1, 100, 4096, 777};
  for (int round = 0; round < 3; ++round) {
    for (int i = 0; i < 4; ++i) {
      CHECK(renoir_frame_loan(pub, sizes[i], &out) == RenoirErrorCode_Success);
      CHECK(renoir_frame_loan(pub, 1, &out) != RenoirErrorCode_Success);
      CHECK(out != NULL);
      memset(out, (int)(i + round), sizes[i]);
      CHECK(renoir_frame_commit(pub) == RenoirErrorCode_Success);
      CHECK(renoir_frame_view(sub, &in, &len) == RenoirErrorCode_Success);
      CHECK(len == sizes[i]);
      for (size_t k = 0; k < len; ++k) {
        CHECK(in[k] == (uint8_t)(i + round));
      }
      CHECK(renoir_frame_view_intact(sub));
      CHECK(renoir_frame_view(sub, &in, &len) != RenoirErrorCode_Success);
      CHECK(renoir_frame_release(sub) == RenoirErrorCode_Success);
      CHECK(renoir_frame_release(sub) != RenoirErrorCode_Success);
    }
  }
  CHECK(renoir_frame_loan(pub, 4097, &out) != RenoirErrorCode_Success);
  CHECK(renoir_frame_loan(pub, 10, &out) == RenoirErrorCode_Success);
  CHECK(renoir_frame_abort(pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_view(sub, &in, &len) == RenoirErrorCode_BufferEmpty);

  for (int i = 0; i < 4; ++i) {
    CHECK(renoir_frame_loan(pub, 64, &out) == RenoirErrorCode_Success);
    memset(out, 0x30 + i, 64);
    CHECK(renoir_frame_commit_notify(pub) == RenoirErrorCode_Success);
  }
  CHECK(renoir_frame_loan(pub, 64, &out) == RenoirErrorCode_BufferFull);
  CHECK(renoir_frame_view(sub, &in, &len) == RenoirErrorCode_Success);
  CHECK(renoir_frame_loan(pub, 64, &out) == RenoirErrorCode_BufferFull);
  CHECK(renoir_frame_release(sub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_loan(pub, 64, &out) == RenoirErrorCode_Success);
  CHECK(renoir_frame_abort(pub) == RenoirErrorCode_Success);
  CHECK(renoir_frame_wait(sub, 10, 1000000) == RenoirErrorCode_Success);
  CHECK(renoir_frame_consumer_id(sub) >= 0);
  CHECK(renoir_frame_consumer_id(pub) == -1);
  CHECK(renoir_frame_live_publishers(pub) >= 1);

  renoir_frame_close(sub);
  renoir_frame_close(pub);
  renoir_frame_close(NULL);
  (void)snprintf(path, sizeof path, "%s/renoir_%s", dir, name);
  (void)unlink(path);
  (void)rmdir(dir);
  puts("frame_abi_test OK");
  return 0;
}
