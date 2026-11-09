# Changelog

All notable changes to Renoir are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to the versioning policy in `SEMVER.md`.

## [Unreleased]

### Added
- Lean rings can carry a publish timestamp: open with `RENOIR_LEAN_STAMP_NS` or `RENOIR_LEAN_STAMP_TICKS` and use `renoir_lean_publish_at`, `renoir_lean_publish_now`, the notifying `renoir_lean_publish_at_notify` and `renoir_lean_publish_now_notify` (needed to wake blocking waiters) and `renoir_lean_try_read_stamped` (Rust: `LeanChannel::open_stamped`, `publish_at`, `publish_now`, `try_read_stamped`). The stamp is 8 bytes inside the slot and is covered by the same sequence check as the payload; the cost over an unstamped publish is about 2 ns with a caller-supplied stamp, 4 ns with the cycle counter and 14 ns with `CLOCK_MONOTONIC`. `renoir_lean_memory_required_flags` sizes stamped rings.
- `renoir_lean_try_read_lagged` and `renoir_lean_try_read_stamped_lagged` (Rust `try_read_lagged`, `try_read_stamped_lagged`) report how many messages a read missed.
- Selectable consumer wait strategies for lean rings: busy-spin, yielding, sleeping, blocking and phased back-off (`renoir_lean_wait_strategy`, `renoir_lean_wait_phased`; Rust `LeanChannel::wait_with`, `wait_phased`), alongside the existing spin-then-futex and adaptive waits.
- Python: `LeanChannel` accepts `clock=` and gains `publish_at`, `publish_now`, `publish_at_notify`, `publish_now_notify`, `try_read_stamped`, `try_read_lagged`, `wait_with`, `wait_phased`, `clock`, `tick_hz`, `stamp_now` and `stamp_to_ns`; `discover()` reports each lean ring's `clock` and its data size, and `DiscoveryWatch` reports rings appearing, changing and disappearing.
- Complete QoS for lean rings (`LeanQos`, `renoir_lean_open_qos`, `LeanQosOptions`, `LeanChannel.with_qos`): keep-all history, transient-local durability, lifespan and time-based filter on stamped rings, manual liveliness with a lease and `register_publisher`/`assert_liveliness`/`liveliness`, exclusive ownership with strength and failover, request/offer compatibility checks at open (optionally required), publishing as a liveliness assertion and a blocking `wait_liveliness_change`. See `docs/src/rpc-qos.md` for the full policy table, including the policies that do not apply to one machine.
- Discovery change notifications: `DiscoveryWatcher` (Rust), `renoir_discover_watch_open/fd/poll/close` (C) and `renoir-monitor events` report rings appearing, changing (publisher or consumer count) and disappearing.

### Changed
- The lean header gained a clock mode and tick rate in space that was padding; the header size and every other offset are unchanged.

### Fixed
- A corrupted or hostile ring header could make a lean or frame ring panic with an arithmetic overflow (in builds with overflow checks) or misbehave silently (in release builds): a write index or multi-producer claim counter of 2^62 or more overflowed `2 * index`. The published index is now clamped at 2^62 where it is read, and the multi-producer claim paths return an error instead of overflowing. Found by a new fuzz target, `lean_hostile_header`, which attaches to a ring whose header and slots are attacker-controlled, including the stamped paths.
- A message sent through the shared buffer pool (`use_shared_pool`) was only resolvable inside the process that published it, because the pool lives in an anonymous in-process mapping. A reader in another process got an empty message reported as success, or, if its own pool happened to hand out the same pool and buffer numbers, a buffer belonging to a different message. Descriptors now carry the identity of the pool manager that issued them; a manager refuses descriptors it did not issue, and the C API returns the new `RenoirErrorCode_UnresolvedPayload` (14) instead of an empty message. Payloads that must cross processes belong on a frame ring (`renoir_frame_*`) or an inline topic.

### Changed
- `MessageDescriptor` gained an `owner_id` field (its size grows from 16 to 24 bytes); the descriptor path is in-process only, so no cross-process data layout changes.

## [0.9.0] - 2026-11-08

### Fixed
- Opening an existing ring with different (smaller) parameters used to truncate the live ring file before the geometry check rejected it, so the process already using the ring would crash with SIGBUS on its next write. Only the process that wins the region's session lock now resizes a file; everyone else gets an error and the file is untouched.
- Rings are created with their pages reserved (`posix_fallocate`) and pre-faulted into each process (`MADV_POPULATE_WRITE`, Linux 5.14+). A full or small `/dev/shm` (Docker's default is 64 MB) now fails at open with a message naming the problem, instead of killing a process with SIGBUS later, and the hot path no longer takes first-touch page faults.
- Removed the last `unwrap`/`expect` calls from library code (buffer layouts in `Drop`, and `SystemTime` before 1970 on boards without a battery-backed clock).
- Registering a gated consumer could, when two consumers raced for the same free table entry, overwrite the winner's position and process fields; entries are now claimed through an intermediate state, and registration reclaims entries whose process has died.
- `lean_slot_bytes` no longer wraps on message sizes close to `usize::MAX` (found by Kani): in release builds such a size was accepted as a one-line slot; it is now rejected.
- Multi-producer rings no longer skip or take over the slot of a producer that is merely slow or suspended: producers register their process id and start time in a 16-entry table (lean header grows from 704 to 1728 bytes, later 3264 with 32 consumer entries), and a claimed slot is skipped only when its owner process is dead or after 10 s.
- Gated multi-producer rings no longer lose a message when a producer is descheduled for more than 5 ms after claiming a slot: the reader now waits up to the gate timeout before skipping a claimed but unpublished slot.

### Fixed
- A ring left half-initialised by a killed process no longer blocks later openers forever: after 2 seconds the next opener takes over the initialisation.
- SPSC and MPMC ring headers are now initialised through a compare-and-swap handshake, removing a data race when two openers created the same ring concurrently.

### Changed
- A gated lean or frame ring now supports 32 registered consumers instead of 8 (the header grows to 3264 bytes).
- Build: `env_logger` is now optional (CLI feature only), `loom` is a `cfg(loom)` dev-dependency, the iceoryx2 comparison benchmarks build from `benches/comparison`, and test runs use a new `testing` profile (no LTO, 16 codegen units). Debug `cargo test --no-run` is about 26% faster and `cargo clippy --all-targets` about 31% faster; clean release-profile test builds drop from 75 s to 19 s. The shipped `release` profile is unchanged.
- The lean ring header grew from 192 to 1728 bytes to hold the consumer and producer tables (wire format change, pre-1.0).
- `librenoir.so` now has SONAME `librenoir.so.0`; the Debian package installs it as `librenoir.so.0` and the Jetson package ships the matching symlinks. `tests/abi/abidiff.sh` compares the built library against a checked-in libabigail baseline in CI.

### Added
- RPC with up to 4 servers per service: each server owns one request shard, clients spread across live shards and fail over when their server dies (`RpcServer::shard`, `RpcClient::shard`, `RPC_MAX_SERVERS`). Before, a second server for the same service silently received every request and produced duplicate replies.
- Frame lifespan: `view_fresh` / `renoir_frame_view_fresh` / `tryViewFresh` / `read(max_age_ms=...)` drop frames older than a limit, using a monotonic stamp written at commit.
- Frame rings (`FrameChannel`, `renoir_frame_*`, C++ `renoir_frame.hpp`, Python `FrameChannel`): dynamic-length frames up to 64 MiB, written in place by the producer (`loan`/`commit`) and read in place by consumers (`view`/`release`) with no copy. Views are owned: the gate holds the slot until release, a live holder is never evicted by a timer, and a holder or loaner that dies is reclaimed through process-liveness checks. Gate entries now record the holder's process id and start time.
- Consumer dependencies on gated lean rings (the Disruptor sequence barrier): `set_upstream` / `consumer_id` (`renoir_lean_set_upstream`, `renoir_lean_consumer_id`, C++ `setUpstream`/`consumerId`, Python `set_upstream`/`consumer_id`). A downstream consumer only sees messages its upstream has already consumed; chains are cycle-checked in shared memory and an upstream that is unregistered or evicted releases its downstream.
- Service discovery and monitoring without a daemon: `renoir::discovery::{scan, sweep}`, `renoir_discover` and `renoir_discover_sweep` in C, and the `renoir-monitor` tool (`list`, `watch`, `sweep`). Rings describe themselves in their headers, so discovery costs nothing at publish time.
- Liveliness: `live_publishers()` on the lean ring (`renoir_lean_live_publishers`, C++ `livePublishers`, Python `live_publishers`) counts registered publisher processes that are still alive, so a consumer can tell a crashed publisher from a quiet one.
- Lean ring: single-writer broadcast ring with one cache line per message of up to 56 bytes (up to 504 bytes across 8 lines), exposed in C as `renoir_lean_*`. Measured about 65-76 ns one-way between pinned cores on x86, against about 250 ns for the broadcast topic ring. Handles are direct pointers with no global registry lock.
- `renoir::LeanChannel` C++ wrapper (`include/renoir_lean.hpp`), Python `renoir.LeanChannel`, and a safe Rust `LeanChannel` type.
- Lean ring blocking consumers: `publish_notify` and `wait` (`renoir_lean_publish_notify`, `renoir_lean_wait`, C++ `publishNotify`/`wait`, Python `publish_notify`/`wait`) using a futex handshake with spin-then-sleep. Plain `publish` is unchanged.
- Lean ring multi-producer mode (`RENOIR_LEAN_MULTI_PRODUCER`, C++ `multiProducer`, Python `multi_producer`, Rust `LeanMode`): claim-and-ticket publishing, hole skipping for dead producers, loom model and SIGKILL chaos test.
- Lean ring gated lossless mode: registered consumers (up to 8) gate the publisher, which returns `BufferFull` instead of overwriting unread messages and evicts consumers that stop advancing for 100 ms. Rust `LeanMode::*Gated`, `LeanGatedCursor`; C `RENOIR_LEAN_GATED` and `RENOIR_LEAN_CONSUMER`; C++ `LeanFlags`; Python `gated` and `consumer`.
- Lean ring batching: `publish_batch` and `read_batch` (`renoir_lean_publish_batch`, `renoir_lean_read_batch`, C++ `publishBatch`/`readBatch`, Python `publish_batch`/`read_batch`).
- Adaptive spin-then-futex wait (`wait_adaptive`, `renoir_lean_wait_adaptive`, C++ `waitAdaptive`, Python `wait_adaptive`) and a sizing calculator (`renoir_lean_memory_required`, Python `lean_memory_required`).
- Request-response (`renoir::rpc::{RpcServer, RpcClient}`, `renoir_rpc_*`, C++ `renoir_rpc.hpp`, Python `RpcServer`/`RpcClient`): one server, many clients, 243 ns round trip for 8-byte payloads on x86.
- In-place frame views for broadcast (SPMC) topics: `BroadcastTopicRing::view`, `renoir_subscribe_view` and `renoir_subscribe_view_release`, C++ `Subscriber::view`/`releaseView`. Consumption cost is independent of payload size (about 160-180 ns at 921,600 bytes); the data is validated after use and reported as overwritten if the producer lapped the slot.
- Documentation: benchmarks, sizing, troubleshooting; `RELEASING.md`, `scripts/repro_build.sh`, `scripts/sbom.sh`.
- QoS profiles for the lean ring (`LeanQos`: reliability, depth, deadline) with `renoir_lean_set_deadline`, `renoir_lean_wait_deadline` and `renoir_lean_deadline_misses`.
- Litmus tests (`tests/litmus`) checking the seqlock and wake handshakes against the AArch64 memory model with herd7, and a SIGKILL publisher chaos test.
- Verification: loom model tests, zero-allocation test, cross-process ordering and first-open race tests, `lean_corrupt_ring` fuzz target, exported-symbol baseline guard (`tests/abi/check.sh`) and `.github/workflows/verify.yml` (Miri, TSan, ASan, loom, fuzz smoke, C ABI, supply chain, aarch64, latency, Python).
- `TopicPattern::SPMC` is now a true broadcast topic: one publisher,
  any number of subscribers, each receiving every message. Slow
  subscribers are overrun (oldest messages skipped, counted via
  `Subscriber::overruns`) and never block the publisher. Payloads are
  inline only, up to `max_payload_size`; `use_shared_pool` is rejected.

- Zero-copy publishing for SPMC topics through the C API: `renoir_publish_reserve`
  returns a pointer into the ring slot, `renoir_publish_commit` publishes it,
  `renoir_publish_discard` abandons it. C++ gets an RAII `Reservation`.
- `renoir_subscribe_read_into` copies the next message straight into a
  caller-owned buffer without allocating; `renoir_subscriber_overruns` reports
  messages a slow subscriber lost.

- Blocking wait for SPMC subscribers using a cross-process futex:
  `renoir_subscribe_wait` and C++ `Subscriber::wait`; timed
  `renoir_subscribe_read_next` no longer polls on SPMC topics. Publishers skip
  the wake syscall when no subscriber is waiting.

- `renoir-cli topics` lists SPMC rings (slots, payload size, published count,
  waiters) by reading only the ring header.

- Shared sensor message layouts for the C ABI: `RenoirImageFrameHeader` (128 B,
  camera intrinsics and stamp in the same message as the pixels) and
  `RenoirImuSample` (128 B), with `renoir_image_frame_*` and `renoir_imu_sample_*`
  helpers that initialise and validate untrusted payloads. Broadcast payloads
  are now 64-byte aligned.

- `renoir_subscriber_skip_to_latest` / C++ `Subscriber::skipToLatest`: drop stale
  unread messages so a slow vision consumer always reads fresh frames.
- `tools/soak/renoir_soak.c`: C-ABI soak and latency test for camera and IMU
  streams.

### Fixed
- `renoir-cli topics` and the ring header parsers no longer panic or read out of
  bounds on corrupt or hostile region files (found by fuzzing).
- A corrupt broadcast slot can no longer make a reader spin forever; it is skipped
  and reported as an overrun.
- `Subscriber::has_new_messages` / `renoir_subscriber_has_messages` and
  `pending_messages` now reflect the shared ring (they used process-local
  statistics and a constant 0).
- Creating the same SPMC topic from several processes at once could fail or
  reset a live ring; header initialisation is now race-free.

- `renoir_topic_register_ex` with `RENOIR_TOPIC_FLAG_FRESH_START` (C++
  `TopicOptions::fresh_start`): discard messages left by a previous clean
  session when no live process holds the topic. `TopicConfig` gained a
  `fresh_start` field.

### Changed
- Topic regions are now created in `$RENOIR_SHM_DIR`, else `/dev/shm`, else
  `/tmp` (previously always `/tmp`), so rings live in RAM instead of being
  written back to disk. `renoir-cli topics` scans the same directory.
- Breaking: SPMC topics use a new ring layout (`BROADCAST_RING_MAGIC`).
  Previously SPMC aliased the SPSC ring, whose single read cursor was
  shared by all subscribers.

## [0.1.0] - 2026-08-15

### Added
- Cross-process pub/sub: topic ring buffers now live inside the topic's
  shared memory region, so publishers and subscribers in different
  processes communicate through the same buffer
- Shared-memory-backed ring control blocks with a documented
  attach/initialize protocol (`from_memory` on both ring types)
- Advisory `flock()`-based region liveness detection: orphaned regions
  from crashed processes are safely reinitialized on next attach
- Real stats/telemetry: end-to-end message latency (average + peak) and
  cumulative throughput are measured and reported through the C API,
  replacing the previous hardcoded-zero fields
- Region-level `used_size` accounting (bytes allocated from a region by
  its buffer pools)
- `renoir-cli topics [--watch]`: introspection of live topic rings in
  shared memory from a separate process
- Python bindings in a separate `renoir-python` crate (PyO3): create
  manager, register topic, publish, subscribe/read-next
- C++ RAII wrapper (`renoir.hpp`) targeting the real FFI surface, with a
  compile-and-run drift guard
- Criterion benchmark for the topic pub/sub hot path with a recorded
  baseline
- Published benchmark comparison against iceoryx2 (see
  `docs/src/benchmarks.md`)
- Named enum variants in the generated C header
  (`RenoirErrorCode_Success`, ...) to avoid global-namespace collisions

### Fixed
- MPMC ring buffer out-of-bounds write/read when a message straddles the
  physical buffer end (wrap-aware copies)
- Torn reads in the MPMC ring: consumers now only observe fully published
  messages (in-order publish watermark)
- Inline/descriptor payload type confusion for short (≤16 byte) messages;
  the wire header now carries an explicit payload-type tag
- `Buffer::write`/`read`, shared-pool size-class lookup, chunk reassembly,
  and FFI topic-list allocation no longer panic or overflow on
  attacker-controlled sizes
- Chunked large-payload storage no longer discards its reassembly handles
  and no longer leaks pool buffers; blob/chunk writes are bounds-checked
  against the pool buffer size
- `renoir_buffer_pool_config` no longer returns a dangling `name` pointer;
  `renoir_topic_list_free` now frees each per-topic name string
- SWMR ring buffer restricted to `Copy` payload types (seqlock soundness)
  and drops overwritten slot values on wraparound
- Version constants derive from `Cargo.toml` (FFI now reports 0.1.0
  consistently)
- Duplicate topic registration with conflicting configuration is now
  rejected instead of silently ignored
- SPSC/SPMC publisher and subscriber attach is now atomic (two racing
  attaches can no longer both succeed)
- Slash-containing topic names (`/sensors/imu`) now get valid
  file-backed region paths
- Removed the dead `structured_layout` module, `mimalloc` dependency, and
  ~60 other dead code items

### Changed
- Topics always use file-backed regions regardless of the notification
  setting, making cross-process attachment the default
- Ring control block wire format: both SPSC and MPMC rings now carry a
  session marker byte at offset 16 of the control block (DIRTY while a
  session is active, CLEAN after a graceful close). This is a breaking
  change per `SEMVER.md` — control blocks written by a previous build are
  not recognized (magic match still holds, but the header grew by 8 bytes
  of padding + marker) and are treated as fresh on next attach; both ends
  must ship the same build
- Region liveness on attach now distinguishes a crashed previous session
  (DIRTY marker: content wiped) from a cleanly closed one (CLEAN marker:
  content preserved). One-shot publishers that publish and exit normally
  no longer have their messages wiped by a later first-time consumer
- `Cargo.toml` package metadata corrected (authors, repository, readme)
- Dual-license grant completed: `LICENSE-MIT` and `LICENSE-APACHE`

### Removed
- `structured_layout` module (superseded by shared-memory rings)
- `mimalloc` / `custom-allocator` feature (no callers)
- Various dead `pub` API items across allocators, buffers, message
  formats, and the FFI surface
