# Changelog

All notable changes to Renoir are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to the versioning policy in `SEMVER.md`.

## [Unreleased]

### Fixed
- SPSC and MPMC ring headers are now initialised through a compare-and-swap handshake, removing a data race when two openers created the same ring concurrently.

### Changed
- The lean ring header grew from 192 to 704 bytes to hold the consumer table (wire format change, pre-1.0).
- `librenoir.so` now has SONAME `librenoir.so.0`; the Debian package installs it as `librenoir.so.0` and the Jetson package ships the matching symlinks. `tests/abi/abidiff.sh` compares the built library against a checked-in libabigail baseline in CI.

### Added
- Lean ring: single-writer broadcast ring with one cache line per message of up to 56 bytes (up to 504 bytes across 8 lines), exposed in C as `renoir_lean_*`. Measured about 65-76 ns one-way between pinned cores on x86, against about 250 ns for the broadcast topic ring. Handles are direct pointers with no global registry lock.
- `renoir::LeanChannel` C++ wrapper (`include/renoir_lean.hpp`), Python `renoir.LeanChannel`, and a safe Rust `LeanChannel` type.
- Lean ring blocking consumers: `publish_notify` and `wait` (`renoir_lean_publish_notify`, `renoir_lean_wait`, C++ `publishNotify`/`wait`, Python `publish_notify`/`wait`) using a futex handshake with spin-then-sleep. Plain `publish` is unchanged.
- Lean ring multi-producer mode (`RENOIR_LEAN_MULTI_PRODUCER`, C++ `multiProducer`, Python `multi_producer`, Rust `LeanMode`): claim-and-ticket publishing, hole skipping for dead producers, loom model and SIGKILL chaos test.
- Lean ring gated lossless mode: registered consumers (up to 8) gate the publisher, which returns `BufferFull` instead of overwriting unread messages and evicts consumers that stop advancing for 100 ms. Rust `LeanMode::*Gated`, `LeanGatedCursor`; C `RENOIR_LEAN_GATED` and `RENOIR_LEAN_CONSUMER`; C++ `LeanFlags`; Python `gated` and `consumer`.
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
  `.ai/benchmarks/iceoryx2_comparison.md`)
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
