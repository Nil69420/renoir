# C++ Binding

`include/renoir.hpp` provides RAII wrappers over the C API, implemented in
`src/cpp/*.cpp`. It is compiled as part of the `renoir_cpp` CMake target.

## Usage

```cpp
#include <renoir.hpp>
#include <cassert>

using namespace renoir;

TopicManager manager;
manager.registerTopic("/sensor/imu", TopicOptions{});

Publisher pub = manager.createPublisher("/sensor/imu");
Subscriber sub = manager.createSubscriber("/sensor/imu");

std::vector<uint8_t> payload = {'h', 'i'};
pub.publish(payload);

ReceivedMessage msg = sub.readNext(1000);
assert(msg.size() == payload.size());
```

## Classes

- `TopicManager`, `Publisher`, `Subscriber`, `ReceivedMessage` — topic
  pub/sub
- `SharedMemoryManager`, `SharedMemoryRegion`, `BufferPool`, `Buffer` —
  regions and buffer pools
- `RingBuffer` — byte ring buffer
- `Exception` / `checkResult` — error handling; every wrapper method
  throws `renoir::Exception` on a non-success error code

## Drift guard

`tests/cmake/wrapper_drift_guard.cpp` includes `renoir.hpp` and exercises
every wrapper class against the real built library. It is wired into the
CMake test suite (`ctest`), so any symbol or layout drift between the
header and the actual FFI surface fails the build loudly.

## Building

The wrapper compiles with the CMake build (`renoir_cpp` target); it links
the Rust shared library built with `--features c-api`.
