# Request-response and QoS

## Request-response

`RpcServer` and `RpcClient` (`renoir_rpc_*` in C, `renoir::RpcServer` and `renoir::RpcClient` in C++, `RpcServer` and `RpcClient` in Python) implement a synchronous call with one server and any number of clients per service.

- Requests travel on a reliable multi-producer lean ring named `<service>_req`; the server is its registered consumer, so a request is never overwritten before the server reads it.
- Each client owns a response ring `<service>_rsp_<client id>`, written only by the server. Responses carry the request sequence number, and a client discards any response that is not for its current call.
- Payload is limited to 488 bytes (the 504-byte lean maximum minus a 16-byte header). Client and server must open the service with the same payload size and request slot count.
- `call` blocks with a spin-then-futex wait until the response arrives or the timeout expires (`RenoirErrorCode_BufferEmpty` in C). A client that calls while no server runs times out; its request stays in the ring and is answered if a server starts and reads it.
- Up to 4 servers per service (`RPC_MAX_SERVERS`). Each server owns one request ring (a shard) exclusively, taking the lowest free shard when it opens, so two servers never answer the same request. Clients spread across the live shards by client id and fail over to another live shard when the server they use dies; a request resent after failover may be processed twice, so make handlers idempotent. A dead server's shard is freed for the next server to open.
- Measured on x86 with pinned cores and a spinning server, round trip p50: 243 ns for 8 bytes, 331 ns for 64 bytes, 395 ns for 256 bytes (`cargo bench --bench rpc_latency`).

## QoS policies

Renoir implements the DDS quality-of-service policy set wherever it means something for processes on one machine, and says so plainly where it does not. Open a lean ring with a policy set through `LeanChannel::open_qos` (Rust), `renoir_lean_open_qos` with a `RenoirLeanQos` (C), `LeanQosOptions` (C++) or `LeanChannel.with_qos(...)` (Python).

| Policy | Support | How it works |
|---|---|---|
| Reliability | yes | best effort: lossy ring, a slow reader is lapped and counts `lost`. Reliable: gated ring with a registered consumer; the publisher gets `BufferFull` instead of overwriting, and a stuck consumer is evicted after 100 ms |
| History | yes | keep last: `depth` slots (at least 2). Keep all: reliable only; the publisher blocks at `depth` (that is the resource limit) |
| Durability | volatile, transient local | volatile: a new reader starts at the newest message. Transient local: it starts at the oldest retained message, so a late joiner receives up to `depth` earlier samples while the ring exists. Persistent is not offered: rings live on tmpfs and do not survive a reboot |
| Deadline | yes | the publisher offers a maximum period, the reader requests one (compatible when the offer is not longer); the reader's `wait_deadline` counts misses |
| Lifespan | yes, on stamped rings | the reader drops samples older than the lifespan and counts them (`stale_dropped`); needs a clock, and assumes stamps come from the ring clock (`publish_now`), not from `publish_at` with another time base |
| Time-based filter | yes, on stamped rings | the reader delivers at most one sample per minimum separation (`filtered_out` counts the rest) |
| Liveliness | automatic and manual | automatic: a publisher counts as alive while its process runs (process id and start time are checked). Manual: it must also assert within the lease, which every publish through a manual-liveliness channel does by itself (about 13 ns of clock read per publish, paid only on those channels) and `assert_liveliness` does explicitly for a publisher that is quiet. A publisher announces itself at startup with `register_publisher` instead of at its first message. Readers call `liveliness()` for the registered and alive counts and a count of times liveliness was lost, or `wait_liveliness_change` (`renoir_lean_wait_liveliness_change`) to block until the alive count or the lost state changes |
| Ownership | shared, exclusive with strength | with exclusive ownership only the live publisher with the highest strength may write; the others get a `Concurrency` error (`RenoirErrorCode_ConcurrencyError`). If the owner dies, or stops asserting within a manual lease, the next strongest takes over after at most about 2 ms plus detection |
| Resource limits | documented | the ring's slots, 16 publishers and 32 consumers per ring |
| Destination order | not applicable | readers see messages in publication order (claim order on multi-producer rings); ordering by source timestamp is not provided, but stamps let a consumer sort |
| Latency budget, transport priority | not applicable | there is no transport to prioritise on one machine |
| Presentation | not applicable | each ring is independent; stamps allow alignment across rings |
| Partition | by naming | use ring names or separate `RENOIR_SHM_DIR` directories |
| User, topic and group data; entity factory; data lifecycle | not applicable | not provided |

Request and offer: the publisher that first opens a ring records what it offers (manual or automatic liveliness and its lease, exclusive or shared ownership, deadline, lifespan) in the ring header. A reader whose request cannot be satisfied is refused at open with an `InvalidParameter` error naming the policy: a deadline shorter than the offered one, a manual lease shorter than the offered one, manual liveliness when the publisher offers automatic, or a different ownership kind. A second publisher that offers something different is refused too. A ring created without QoS (plain `open`) records no offer and accepts any request. Reliability is checked by the ring mode, which must match.

```c
RenoirLeanQos qos;
renoir_lean_qos_default(&qos);
qos.clock = RENOIR_LEAN_CLOCK_NS;
qos.lifespan_ns = 50000000;          /* drop samples older than 50 ms */
qos.min_separation_ns = 10000000;    /* at most one sample per 10 ms */
qos.liveliness_kind = RENOIR_QOS_LIVELINESS_MANUAL;
qos.liveliness_lease_ns = 200000000; /* assert within 200 ms */
renoir_lean_open_qos("imu", sizeof(struct imu), &qos, 0, &pub);
renoir_lean_register_publisher(pub);
renoir_lean_publish_now(pub, &sample, sizeof sample);
renoir_lean_assert_liveliness(pub);  /* from the publisher's heartbeat */

renoir_lean_open_qos("imu", sizeof(struct imu), &qos, RENOIR_LEAN_CONSUMER, &sub);
RenoirLivelinessStatus status;
renoir_lean_liveliness(sub, &status);        /* status.lost, status.alive */
renoir_lean_try_read_stamped_qos(sub, buf, sizeof buf, &stamp);
```

Costs, measured on an i7-11800H, single thread, best of 7: a normal publish and read is 8 ns, and the policies add nothing to it (a before-and-after comparison of the ping-pong round trip showed no change). A stamped publish and read is 22 ns, 35 ns with a lifespan and filter active (it reads the clock for each message). An owner's exclusive publish is about 39 ns (it is a multi-producer publish plus a clock read; a non-owner's attempt is refused without writing). `assert_liveliness` is 13 ns. `liveliness()` and the wait take microseconds (3.4 us with one publisher) because it asks the kernel whether each publisher process is alive, so poll it at tens of hertz, not per message.

Deviations from DDS to know about: liveliness is per publishing process and per publisher object, not per topic; `wait_liveliness_change` polls the kernel for process liveness at 1 to 8 ms intervals, so it notices a change within about 8 ms, not instantly; publishing through the raw `LeanPublisher` (not a `LeanChannel`) does not assert liveliness; ownership applies to rings opened with exclusive ownership (which are multi-producer rings) and is re-evaluated at most every 2 ms, so for up to about 2 ms after a stronger publisher appears the previous owner may still write; a ring that records no offer accepts any request unless the reader sets `require_offer` (`RENOIR_QOS_FLAG_REQUIRE_OFFER`, `requireOffer`), which refuses it. The policy set was taken from the DDS documentation of several implementations (RTI, RustDDS, OpenDDS), not from the normative OMG specification.

Frame rings keep their own lifespan: each frame is stamped with a monotonic microsecond clock at commit (`FrameChannel::view_fresh`, `renoir_frame_view_fresh`, C++ `tryViewFresh`, Python `read(max_age_ms=...)`); frames older than the limit are released and counted in `stale_dropped`. Ages up to about 35 minutes are exact.
