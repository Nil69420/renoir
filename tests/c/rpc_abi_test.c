#define _POSIX_C_SOURCE 200809L
#include <pthread.h>
#include <stdatomic.h>
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

struct server_ctx {
  RenoirRpcHandle server;
  atomic_int stop;
  uint64_t served;
};

static void *serve(void *arg) {
  struct server_ctx *ctx = arg;
  uint8_t buf[64];
  while (!atomic_load(&ctx->stop)) {
    uint32_t client = 0;
    uint64_t seq = 0;
    size_t len = 0;
    if (renoir_rpc_poll(ctx->server, buf, sizeof buf, &client, &seq, &len) ==
        RenoirErrorCode_Success) {
      for (size_t i = 0; i < len; ++i) {
        buf[i] = (uint8_t)(buf[i] + 1U);
      }
      if (renoir_rpc_reply(ctx->server, client, seq, buf, len) == RenoirErrorCode_Success) {
        ctx->served++;
      }
    } else {
      (void)renoir_rpc_wait_request(ctx->server, 100, 2000000);
    }
  }
  return NULL;
}

int main(void) {
  char dir[] = "/tmp/renoir_rpc_XXXXXX";
  char service[64];
  CHECK(mkdtemp(dir) != NULL);
  CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
  (void)snprintf(service, sizeof service, "rpc_abi_c_%d", (int)getpid());

  RenoirRpcHandle server = NULL;
  RenoirRpcHandle client = NULL;
  CHECK(renoir_rpc_server_open(NULL, 32, 16, &server) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_rpc_server_open(service, 0, 16, &server) == RenoirErrorCode_InvalidParameter);
  CHECK(renoir_rpc_server_open(service, 32, 16, &server) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_client_open(service, 32, 16, &client) == RenoirErrorCode_Success);

  struct server_ctx ctx = {server, 0, 0};
  pthread_t thread;
  CHECK(pthread_create(&thread, NULL, serve, &ctx) == 0);

  uint8_t request[8] = {1, 2, 3, 4, 5, 6, 7, 8};
  uint8_t response[32];
  size_t response_len = 0;
  for (int i = 0; i < 200; ++i) {
    request[0] = (uint8_t)i;
    CHECK(renoir_rpc_call(client, request, sizeof request, response, sizeof response, 5000000000ULL,
                          &response_len) == RenoirErrorCode_Success);
    CHECK(response_len == sizeof request);
    CHECK(response[0] == (uint8_t)(i + 1) && response[7] == 9);
  }
  CHECK(renoir_rpc_call(client, request, 33, response, sizeof response, 1000000ULL,
                        &response_len) != RenoirErrorCode_Success);
  CHECK(renoir_rpc_call(NULL, request, 8, response, sizeof response, 1000ULL, &response_len) ==
        RenoirErrorCode_InvalidParameter);

  atomic_store(&ctx.stop, 1);
  CHECK(pthread_join(thread, NULL) == 0);
  CHECK(ctx.served == 200);

  RenoirRpcHandle lonely = NULL;
  char other[64];
  (void)snprintf(other, sizeof other, "rpc_abi_c_nosrv_%d", (int)getpid());
  CHECK(renoir_rpc_client_open(other, 32, 16, &lonely) == RenoirErrorCode_Success);
  CHECK(renoir_rpc_call(lonely, request, 8, response, sizeof response, 20000000ULL,
                        &response_len) == RenoirErrorCode_BufferEmpty);

  renoir_rpc_client_close(lonely);
  renoir_rpc_client_close(client);
  renoir_rpc_server_close(server);
  renoir_rpc_client_close(NULL);
  renoir_rpc_server_close(NULL);
  puts("rpc_abi_test OK");
  return 0;
}
