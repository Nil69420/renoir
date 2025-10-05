# Topic System Examples

This directory contains comprehensive examples demonstrating Renoir's topic-based pub/sub messaging system.

## Examples

### 1. Basic Topic Example (`topic_basic_example.cpp`)

**Purpose**: Introduction to topic fundamentals

**Features**:
- Creating a topic manager
- Registering topics with SPSC ring buffers
- Creating publishers and subscribers  
- Publishing and receiving simple messages
- Retrieving topic statistics
- Proper resource cleanup

**Run**:
```bash
cd examples/build
LD_LIBRARY_PATH=../../target/release ./topic_basic_example
```

**Output**:
```
=== Renoir Topic Basic Example ===
1. Creating topic manager...
   ✓ Topic manager created
2. Registering topic '/example/messages'...
   ✓ Topic registered (ID: 691509894)
3. Creating publisher...
   ✓ Publisher created
...
=== Example Completed Successfully! ===
```

### 2. Async Event-Driven Example (`topic_async_example.cpp`)

**Purpose**: Demonstrates event-driven messaging with epoll (Linux)

**Features**:
- Event file descriptor (eventfd) integration
- Non-blocking message reads
- epoll-based event loop
- Multi-threaded producer/consumer
- Efficient async notification system

**Run**:
```bash
cd examples/build
LD_LIBRARY_PATH=../../target/release ./topic_async_example
```

**Key Concepts**:
- Uses `renoir_subscriber_get_event_fd()` to get eventfd
- Integrates with epoll for efficient event notification
- Non-blocking reads with `renoir_subscribe_read_next(subscriber, &msg, 0)`
- Falls back to polling on non-Linux platforms

### 3. Comprehensive Integration Example (`topic_comprehensive_example.cpp`)

**Purpose**: Full-featured example showing topics + control region + ring buffers integration

**Features**:
- Multiple topic types (SPSC and MPMC)
- Structured message types (IMU sensor data)
- Multi-threaded pub/sub patterns
- Control region metadata tracking
- Ring buffer statistics monitoring
- Event-driven consumer thread
- Comprehensive error handling

**Run**:
```bash
cd examples/build
LD_LIBRARY_PATH=../../target/release ./topic_comprehensive_example
```

**Output Example**:
```
>>> Running Multi-threaded Pub/Sub Test
  [Producer] Published IMU #0 (seq=1)
  [Consumer] Received IMU (seq=0): accel=[9.810, 0.000, 0.000]
  
>>> Topic and Ring Buffer Statistics
  /sensors/imu Statistics:
    Messages Published: 10
    Messages Consumed: 10
    Messages Dropped: 0
```

## Building Examples

### Using build.sh (Recommended)
```bash
# Build everything (Rust + examples)
./build.sh

# Build only examples
./build.sh build-examples

# Clean and rebuild
./build.sh clean
./build.sh
```

### Using CMake directly
```bash
cd examples
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release
make
```

## Running Tests

```bash
cd examples
./test.sh
```

This runs all examples including topic examples and reports pass/fail status.

## API Overview

### Topic Manager
```cpp
RenoirTopicManagerHandle manager = renoir_topic_manager_create();
```

### Topic Registration
```cpp
RenoirTopicOptions opts = {};
opts.pattern = 0;  // 0=SPSC, 1=SPMC, 2=MPSC, 3=MPMC
opts.ring_capacity = 32;  // Must be power of 2
opts.max_payload_size = 1024;
opts.enable_notifications = true;

RenoirTopicId topic_id;
renoir_topic_register(manager, "/topic/name", &opts, &topic_id);
```

### Publisher
```cpp
RenoirPublisherHandle publisher = nullptr;
renoir_publisher_create(manager, "/topic/name", &pub_opts, &publisher);

RenoirMessageMetadata metadata = {};
metadata.timestamp_ns = now();
metadata.sequence_number = seq;

RenoirSequenceNumber seq_out;
renoir_publish(publisher, data, size, &metadata, &seq_out);
```

### Subscriber
```cpp
RenoirSubscriberHandle subscriber = nullptr;
renoir_subscriber_create(manager, "/topic/name", &sub_opts, &subscriber);

RenoirReceivedMessage msg = {};
renoir_subscribe_read_next(subscriber, &msg, timeout_ms);

// Access message
const uint8_t* data = msg.payload_ptr;
size_t len = msg.payload_len;
uint64_t seq = msg.metadata.sequence_number;

// Release when done
renoir_message_release(msg.handle);
```

### Event-Driven (Linux)
```cpp
int event_fd;
renoir_subscriber_get_event_fd(subscriber, &event_fd);

// Use with epoll, select, or poll
struct epoll_event ev;
ev.events = EPOLLIN;
ev.data.fd = event_fd;
epoll_ctl(epoll_fd, EPOLL_CTL_ADD, event_fd, &ev);

// When event triggers, read messages
renoir_subscribe_read_next(subscriber, &msg, 0);  // Non-blocking
```

## Important Notes

1. **Ring Capacity**: Must be a power of 2 (2, 4, 8, 16, 32, 64, etc.)
2. **Thread Safety**: All operations are thread-safe
3. **Resource Cleanup**: Always destroy subscribers before publishers
4. **Memory Management**: Call `renoir_message_release()` for each received message
5. **Pattern Selection**:
   - SPSC (0): Single producer, single consumer - fastest
   - SPMC (1): Single producer, multiple consumers
   - MPSC (2): Multiple producers, single consumer
   - MPMC (3): Multiple producers, multiple consumers - most flexible

## Integration with ROS2

These examples demonstrate patterns suitable for ROS2 integration:

- **Topic-based messaging** similar to ROS2 topics
- **Event-driven subscriptions** for efficient spinning
- **Multi-threaded patterns** for executors
- **Statistics tracking** for debugging and monitoring

## Performance Considerations

- Use SPSC pattern when possible for best performance
- Enable notifications for event-driven operation
- Use appropriate ring capacity (balance memory vs latency)
- Consider message sizes relative to ring capacity
- Monitor statistics for dropped messages

## Troubleshooting

**Error: "Capacity must be a power of 2"**
- Solution: Use 2, 4, 8, 16, 32, 64, 128, etc. for ring_capacity

**Error: "Failed to create topic manager"**
- Check shared memory permissions
- Ensure /dev/shm is writable

**Messages getting dropped**
- Increase ring_capacity
- Speed up consumer processing
- Check subscriber statistics

## See Also

- `/examples/buffers/` - Buffer pool and ring buffer examples
- `/examples/region_manager/` - Shared memory region examples
- `TOPIC_FFI_DESIGN.md` - Complete FFI API design document
