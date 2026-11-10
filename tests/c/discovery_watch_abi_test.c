#define _POSIX_C_SOURCE 200809L
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
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

static int next_event(RenoirDiscoveryWatchHandle watch, uint32_t kind, const char *name,
                      RenoirDiscoveryEvent *found) {
  for (int attempt = 0; attempt < 100; ++attempt) {
    RenoirDiscoveryEvent events[2];
    size_t count = 0;
    if (renoir_discover_watch_poll(watch, 50, events, 2, &count) != RenoirErrorCode_Success) {
      return 0;
    }
    for (size_t i = 0; i < count; ++i) {
      if (events[i].kind == kind && strcmp(events[i].ring.name, name) == 0) {
        *found = events[i];
        return 1;
      }
    }
  }
  return 0;
}

int main(void) {
  char dir[] = "/tmp/renoir_watch_XXXXXX";
  char name[64];
  char path[160];
  CHECK(mkdtemp(dir) != NULL);
  CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
  (void)snprintf(name, sizeof name, "watch_abi_c_%d", (int)getpid());
  (void)snprintf(path, sizeof path, "%s/renoir_%s", dir, name);

  RenoirDiscoveryWatchHandle watch = NULL;
  RenoirDiscoveryEvent event;
  size_t count = 0;
  CHECK(renoir_discover_watch_open(0, &watch) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_discover_watch_open(20, NULL) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_discover_watch_open(20, &watch) == RenoirErrorCode_Success);
  CHECK(renoir_discover_watch_fd(NULL) == -1);
  CHECK(renoir_discover_watch_poll(NULL, 0, &event, 1, &count) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_discover_watch_poll(watch, 0, NULL, 1, &count) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_discover_watch_poll(watch, 0, &event, 1, NULL) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_discover_watch_poll(watch, 10, &event, 1, &count) == RenoirErrorCode_Success);
  CHECK(count == 0);

  int fd = renoir_discover_watch_fd(watch);
  CHECK(fd >= 0);
  struct pollfd pfd = {fd, POLLIN, 0};
  CHECK(poll(&pfd, 1, 0) == 0);

  RenoirLeanHandle pub = NULL;
  RenoirLeanHandle sub = NULL;
  CHECK(renoir_lean_open(name, 24, 64, RENOIR_LEAN_RELIABLE_PUBLISHER, &pub) == RenoirErrorCode_Success);
  CHECK(poll(&pfd, 1, 1000) == 1);
  CHECK(next_event(watch, RENOIR_DISCOVERY_APPEARED, name, &event));
  CHECK(event.ring.kind == RENOIR_RING_KIND_LEAN && event.ring.msg_size == 24);

  CHECK(renoir_lean_open(name, 24, 64, RENOIR_LEAN_RELIABLE_SUBSCRIBER, &sub) ==
        RenoirErrorCode_Success);
  CHECK(next_event(watch, RENOIR_DISCOVERY_CHANGED, name, &event));
  CHECK(event.previous_registered_consumers == 0 && event.ring.registered_consumers == 1);

  renoir_lean_close(sub);
  renoir_lean_close(pub);
  CHECK(unlink(path) == 0);
  CHECK(next_event(watch, RENOIR_DISCOVERY_DISAPPEARED, name, &event));

  renoir_discover_watch_close(watch);
  renoir_discover_watch_close(NULL);
  (void)rmdir(dir);
  puts("discovery watch ABI ok");
  return 0;
}
