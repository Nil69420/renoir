# Renoir

**Shared-memory messaging for real-time robotics: about 70 ns for small messages, about 100 ns for frames up to 64 MiB, no daemon.**

Renoir moves sensor data between processes on one machine through shared memory. It is a Rust library with a stable C ABI, a C++ wrapper and Python bindings. It targets flight controllers, perception pipelines and simulators where an IMU sample, a pose or a camera frame has to reach another process without a copy, a lock or a system call on the hot path.

| | |
|---|---|
| **Small messages** | 66 ns one-way for 56 bytes, on the same order as the hardware's cache-line transfer floor (66 ns) |
| **Frames** | 95 to 113 ns one-way from 56 bytes to 900 KB, written in place by the producer and read in place by consumers |
| **Request / response** | 243 ns round trip for 8 bytes |
| **Hot path** | No allocation, lock, system call or panic, proven by a counting-allocator test and by reading the generated code |
| **Deployment** | One shared library (`librenoir.so.0`), no broker, no daemon, no ROS dependency |

Numbers are one-way medians between two pinned cores of an i7-11800H. Method, caveats and the iceoryx2 comparison are in [docs/src/benchmarks.md](docs/src/benchmarks.md). All figures are from x86; Jetson numbers have not been measured by the project.

## What it provides

- **Lean rings** for small fixed-size messages (up to 504 bytes): IMU samples, poses, commands. Lossy by default, optionally gated for lossless delivery with consumer eviction, multi-producer, batched, with the DDS quality-of-service policy set that applies to one machine (reliability, history, durability, deadline, lifespan, time-based filter, liveliness, exclusive ownership with failover, request/offer checks) and consumer dependencies. Messages can carry a publish timestamp (caller-supplied, `CLOCK_MONOTONIC` or the raw cycle counter, 2 to 14 ns extra), a read can report how many messages it missed, and consumers choose a wait strategy (busy-spin, yielding, sleeping, blocking, phased back-off).
- **Frame rings** for large and variable-size data (1 byte to 64 MiB): camera images, point clouds. Zero-copy on both sides, owned until released, lossless, with freshness filtering so a slow consumer skips stale frames.
- **Request / response** (`renoir_rpc_*`): up to four servers per service with client failover.
- **Topics** with SPSC, SPMC, MPSC and MPMC patterns, for general-purpose pub/sub.
- **Discovery and operations**: `renoir-monitor` lists and cleans up rings without a daemon, and reports rings appearing, changing and disappearing (`renoir-monitor events`, `renoir_discover_watch_*`). Crashed producers and consumers are detected by process id and start time and reclaimed.
- **Bindings**: C (`renoir.h`), C++ (`renoir.hpp`, `renoir_lean.hpp`, `renoir_frame.hpp`), Python (`renoir-python`), Rust.

## Quick start

Build the library and header:

```sh
cargo build --release --features c-api
```

This produces `target/release/librenoir.so.0` and `include/renoir.h`.

A lean ring between two processes (C):

```c
#include <renoir.h>

RenoirLeanHandle pub, sub;
renoir_lean_open("imu", sizeof(struct imu), 1024, 0, &pub);
renoir_lean_open("imu", sizeof(struct imu), 1024, RENOIR_LEAN_CONSUMER, &sub);

renoir_lean_publish(pub, (const uint8_t *)&sample, sizeof sample);

struct imu out;
while (renoir_lean_try_read(sub, (uint8_t *)&out, sizeof out) == RenoirErrorCode_Success) {
    /* use out */
}
renoir_lean_close(sub);
renoir_lean_close(pub);
```

A frame ring for camera images (C), written and read in place:

```c
RenoirFrameHandle prod, cons;
renoir_frame_open("camera", 1280 * 720, 8, 0, &prod);
renoir_frame_open("camera", 1280 * 720, 8, RENOIR_LEAN_CONSUMER, &cons);

uint8_t *dst;
renoir_frame_loan(prod, 1280 * 720, &dst);
render_into(dst);
renoir_frame_commit(prod);

const uint8_t *src;
uintptr_t len;
if (renoir_frame_view(cons, &src, &len) == RenoirErrorCode_Success) {
    process(src, len);
    renoir_frame_release(cons);
}
```

Rust and Python:

```rust
let mut producer = FrameChannel::open("camera", 921_600, 8, false, false, false)?;
let mut loan = producer.loan(640 * 480 * 3)?;
fill_image(loan.bytes_mut());
loan.commit()?;
```

```python
import renoir_python as renoir

mgr = renoir.TopicManager()
mgr.create_topic("my_topic", pattern="spsc", ring_capacity=64, max_payload_size=1024)
pub = mgr.create_publisher("my_topic")
sub = mgr.create_subscriber("my_topic")
pub.publish(b"hello")
print(sub.read_next(timeout_ms=1000))
```

The full tutorial, run literally against this repository, is [docs/src/getting-started.md](docs/src/getting-started.md).

## Choosing a transport

| Data | Use | Why |
|---|---|---|
| IMU, pose, small commands (up to 504 B) | Lean ring | One cache line per message, polling path about 70 ns |
| Camera frames, point clouds, anything large or variable | Frame ring | In-place write and read, lossless, owned until released |
| Calls that need an answer | `renoir_rpc_*` | About 250 ns round trip, failover between servers |
| General pub/sub with many patterns | Topics | SPSC, SPMC, MPSC, MPMC |

Large payloads that must cross a process boundary belong on a frame ring or an inline topic. The shared buffer pool behind `use_shared_pool` resolves buffers only inside the process that published them; a reader in another process receives `RenoirErrorCode_UnresolvedPayload`.

## How it is verified

The evidence for each claim, and the list of what is not verified, is in [docs/src/production-readiness.md](docs/src/production-readiness.md). In short:

| Area | Tooling |
|---|---|
| Undefined behaviour | Miri (default and Tree Borrows), UBSan, AddressSanitizer |
| Data races and ordering | ThreadSanitizer, loom, shuttle, herd7 litmus tests on the AArch64 model, native arm64 CI |
| Protocols | TLA+ models of the multi-producer ring and frame ownership, Kani proofs of the sizing arithmetic |
| Hostile or corrupt shared memory | cargo-fuzz targets for lean, broadcast and frame rings |
| Crash recovery | Chaos tests that SIGKILL publishers, consumers, frame holders and RPC servers under load |
| Interface stability | SONAME `librenoir.so.0`, exported-symbol baseline, libabigail comparison in CI |
| Supply chain | cargo-deny, cargo-audit, SBOM, reproducible build |

Known limits: no Jetson measurements, no third-party security review, no MISRA or CERT compliance claim, and liveness detection needs processes to share a pid namespace. See the readiness page for the full list.

## Build and test

```sh
./build.sh                 # Rust library, C headers, C++ examples
./build.sh test            # test suite
cargo clippy --all-targets --all-features -- -D warnings
cargo test
scripts/ci_local.sh        # the verification suite the CI runs, from a clean checkout
```

| Feature flag | Default | Description |
|---|---|---|
| `file-backed` | yes | File-backed shared memory regions |
| `memfd` | yes | `memfd_create` support (Linux) |
| `c-api` | yes | C FFI bindings via `cbindgen` |
| `cli` | no | Command-line tools (`renoir-monitor`) |

Cross-compile for ARM:

```sh
cargo build --release --target aarch64-unknown-linux-gnu     # Jetson, Raspberry Pi 4
cargo build --release --target armv7-unknown-linux-gnueabihf # Raspberry Pi 3
```

Requirements: Linux (uses `memfd_create`, `eventfd`, futexes), Rust 2021 edition, CMake for the C++ examples. Targets: x86_64, aarch64, armv7.

## Documentation

- [Getting started](docs/src/getting-started.md)
- [Lean ring](docs/src/lean-ring.md) and [frame rings](docs/src/frames.md)
- [Request / response and QoS](docs/src/rpc-qos.md)
- [Discovery](docs/src/discovery.md) and [sizing](docs/src/sizing.md)
- [Benchmarks](docs/src/benchmarks.md) and [performance notes](docs/src/performance.md)
- [Production readiness](docs/src/production-readiness.md) and [troubleshooting](docs/src/troubleshooting.md)
- [Security policy](SECURITY.md), [versioning policy](SEMVER.md), [release process](RELEASING.md), [changelog](CHANGELOG.md)
- [Examples](examples/) for C++ programs covering topics, regions and buffers

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
