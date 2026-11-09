# Request-response and QoS

## Request-response

`RpcServer` and `RpcClient` (`renoir_rpc_*` in C, `renoir::RpcServer` and `renoir::RpcClient` in C++, `RpcServer` and `RpcClient` in Python) implement a synchronous call with one server and any number of clients per service.

- Requests travel on a reliable multi-producer lean ring named `<service>_req`; the server is its registered consumer, so a request is never overwritten before the server reads it.
- Each client owns a response ring `<service>_rsp_<client id>`, written only by the server. Responses carry the request sequence number, and a client discards any response that is not for its current call.
- Payload is limited to 488 bytes (the 504-byte lean maximum minus a 16-byte header). Client and server must open the service with the same payload size and request slot count.
- `call` blocks with a spin-then-futex wait until the response arrives or the timeout expires (`RenoirErrorCode_BufferEmpty` in C). A client that calls while no server runs times out; its request stays in the ring and is answered if a server starts and reads it.
- Up to 4 servers per service (`RPC_MAX_SERVERS`). Each server owns one request ring (a shard) exclusively, taking the lowest free shard when it opens, so two servers never answer the same request. Clients spread across the live shards by client id and fail over to another live shard when the server they use dies; a request resent after failover may be processed twice, so make handlers idempotent. A dead server's shard is freed for the next server to open.
- Measured on x86 with pinned cores and a spinning server, round trip p50: 243 ns for 8 bytes, 331 ns for 64 bytes, 395 ns for 256 bytes (`cargo bench --bench rpc_latency`).

## QoS profiles

`LeanQos` maps the policies a lean ring can honour at no hot-path cost:

| Policy | Meaning on the lean ring |
|---|---|
| Reliability: best effort | lossy ring; a slow reader is lapped and counts `lost` |
| Reliability: reliable | gated ring with a registered consumer; the publisher gets `BufferFull` instead of overwriting; stuck consumers are evicted after 100 ms |
| History depth | number of slots (at least 2) |
| Deadline | consumer-side: `wait_deadline` waits at most the deadline for the next message and counts a miss when none arrives |

In C use `RENOIR_LEAN_RELIABLE_PUBLISHER` and `RENOIR_LEAN_RELIABLE_SUBSCRIBER` when opening, and `renoir_lean_set_deadline`, `renoir_lean_wait_deadline` and `renoir_lean_deadline_misses` for deadlines.

Lifespan: supported on frame rings, where each frame is stamped with a monotonic microsecond clock at commit (`FrameChannel::view_fresh`, `renoir_frame_view_fresh`, C++ `tryViewFresh`, Python `read(max_age_ms=...)`); frames older than the limit are released and counted in `stale_dropped`. Ages up to about 35 minutes are exact. It is not available on the lean ring, whose 100 ns path cannot afford a clock read per publish: put a timestamp in your own message if you need it there.

Liveliness: `live_publishers()` counts the registered publisher processes that are still alive (process id and start time are checked), so a consumer can tell a crashed publisher from a quiet one; there is no heartbeat lease or manual assertion.
