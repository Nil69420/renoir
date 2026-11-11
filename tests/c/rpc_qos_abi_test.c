#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
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

int main(void) {
  char dir[] = "/tmp/renoir_rpcq_XXXXXX";
  char service[64];
  CHECK(mkdtemp(dir) != NULL);
  CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
  (void)snprintf(service, sizeof service, "rpc_qos_c_%d", (int)getpid());

  RenoirRpcHandle client = NULL;
  RenoirRpcHandle server = NULL;
  uint8_t response[32];
  uint8_t buf[32];
  uintptr_t out_len = 0;
  uint32_t client_id = 0;
  uint64_t sequence = 0;

  CHECK(renoir_rpc_client_open(service, 32, 16, &client) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_client_servers_alive(client) == 0);
  CHECK(renoir_rpc_client_servers_alive(NULL) == -1);
  CHECK(renoir_rpc_client_wait_for_server(client, 20 * MS) == RenoirErrorCode_BufferEmpty);
  CHECK(renoir_rpc_client_wait_for_server(NULL, 1) == RenoirErrorCode_InvalidParameter);

  CHECK(renoir_rpc_server_open(service, 32, 16, &server) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_client_wait_for_server(client, 2000 * MS) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_client_servers_alive(client) == 1);
  CHECK(renoir_rpc_server_set_lifespan(server, 50 * MS) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_server_set_lifespan(NULL, 1) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_rpc_client_set_stamp(NULL, 1) == RenoirErrorCode_InvalidParameter);

  CHECK(renoir_rpc_client_set_stamp(client, 1) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_call(client, (const uint8_t *)"old", 3, response, sizeof response, 20 * MS, &out_len) ==
        RenoirErrorCode_BufferEmpty);
  CHECK(renoir_rpc_client_set_stamp(client, 0) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_call(client, (const uint8_t *)"plain", 5, response, sizeof response, 20 * MS, &out_len) ==
        RenoirErrorCode_BufferEmpty);
  sleep_ms(120);

  size_t len = 0;
  CHECK(renoir_rpc_poll(server, buf, sizeof buf, &client_id, &sequence, &len) == RenoirErrorCode_Success);
  CHECK(len == 5 && memcmp(buf, "plain", 5) == 0);
  CHECK(renoir_rpc_server_stale_requests(server) == 1);
  CHECK(renoir_rpc_poll(server, buf, sizeof buf, &client_id, &sequence, &len) != RenoirErrorCode_Success);
  CHECK(renoir_rpc_server_stale_requests(NULL) == 0);

  renoir_rpc_client_close(client);
  renoir_rpc_server_close(server);
  puts("rpc qos ABI ok");
  return 0;
}
