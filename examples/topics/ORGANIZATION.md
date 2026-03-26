# Topic Examples - Organization and Structure

## What Was Fixed

### ❌ Previous Issues
1. **Poor Organization**: Created standalone `examples/cpp/` folder that didn't follow existing structure
2. **No CMake Integration**: Examples had their own isolated CMakeLists.txt not integrated with main build
3. **Ignored Existing Build System**: Didn't use `build.sh` or `test.sh` scripts
4. **Wrong Function Names**: Used `renoir_subscribe_try_read()` which doesn't exist
5. **Incomplete Examples**: Basic examples only, no integration with control regions or ring buffers

### ✅ Fixed Structure

```
renoir/
├── build.sh              # Main build script (unchanged, works correctly)
├── examples/
│   ├── CMakeLists.txt    # Updated to include topics/
│   ├── test.sh           # Updated to test topic examples
│   ├── buffers/          # Existing examples (unchanged)
│   ├── region_manager/   # Existing examples (unchanged)
│   └── topics/           # NEW - properly organized
│       ├── CMakeLists.txt
│       ├── README.md
│       ├── topic_basic_example.cpp
│       ├── topic_async_example.cpp
│       └── topic_comprehensive_example.cpp
```

## Topic Examples

### 1. Basic Example (`topic_basic_example.cpp`)
- **Lines**: 140
- **Purpose**: Introduction to topic pub/sub
- **Features**: Manager creation, topic registration, pub/sub, statistics
- **Status**: ✅ Compiles and runs successfully

### 2. Async Example (`topic_async_example.cpp`)
- **Lines**: 180
- **Purpose**: Event-driven messaging with epoll
- **Features**: eventfd integration, non-blocking reads, epoll event loop, multi-threading
- **Status**: ✅ Compiles and runs successfully
- **Platform**: Linux (with fallback for other platforms)

### 3. Comprehensive Example (`topic_comprehensive_example.cpp`)
- **Lines**: 358
- **Purpose**: Full integration demo
- **Features**:
  - Multiple topic types (SPSC, MPMC)
  - Structured message types (IMU sensor data)
  - Multi-threaded producer/consumer
  - Topic and ring buffer statistics
  - Control region metadata tracking
  - Event-driven notifications
- **Status**: ✅ Compiles and runs successfully

## Build System Integration

### Build Commands
```bash
# Build everything
./build.sh

# Build only Rust library
./build.sh build-rust

# Build only examples
./build.sh build-examples

# Clean everything
./build.sh clean
```

### Test All Examples
```bash
cd examples
./test.sh
```

**Output**:
```
================================
 Renoir Examples Test Suite
================================

Running tests...

Testing: Region Manager Example
✓ PASSED: Region Manager Example
Testing: Buffer Pool Example
✓ PASSED: Buffer Pool Example
Testing: Ring Buffer Example
✓ PASSED: Ring Buffer Example
Testing: Control Region Example
✓ PASSED: Control Region Example
Testing: Topic Basic Example
✓ PASSED: Topic Basic Example
Testing: Topic Async Example
✓ PASSED: Topic Async Example
Testing: Topic Comprehensive Example
✓ PASSED: Topic Comprehensive Example

================================
 Test Summary
================================
Total Tests: 7
Passed: 7
Failed: 0

✓ All tests passed!
```

## API Corrections

### Fixed Function Names
- ❌ `renoir_subscribe_try_read()` - Does not exist
- ✅ `renoir_subscribe_read_next(subscriber, &msg, 0)` - Use timeout=0 for non-blocking

### Correct Usage Patterns

**Blocking Read**:
```cpp
renoir_subscribe_read_next(subscriber, &msg, 1000);  // 1 second timeout
```

**Non-blocking Read**:
```cpp
renoir_subscribe_read_next(subscriber, &msg, 0);  // Immediate return
```

**Event-Driven Pattern** (Linux):
```cpp
int event_fd;
renoir_subscriber_get_event_fd(subscriber, &event_fd);

// Setup epoll
int epoll_fd = epoll_create1(0);
struct epoll_event ev;
ev.events = EPOLLIN;
ev.data.fd = event_fd;
epoll_ctl(epoll_fd, EPOLL_CTL_ADD, event_fd, &ev);

// Wait for events
epoll_wait(epoll_fd, events, 1, timeout_ms);

// Read messages when notified
while (true) {
    RenoirReceivedMessage msg = {};
    auto result = renoir_subscribe_read_next(subscriber, &msg, 0);
    if (result == RenoirErrorCode::BufferEmpty) break;
    
    // Process message
    renoir_message_release(msg.handle);
}

// Clear notification
uint64_t buf;
read(event_fd, &buf, sizeof(buf));
```

## Following Existing Patterns

The topic examples now follow the same structure as existing examples:

1. **Each subsystem has its own directory** (`buffers/`, `region_manager/`, `topics/`)
2. **Each directory has CMakeLists.txt** defining its examples
3. **Main CMakeLists.txt includes all subdirectories** via `add_subdirectory()`
4. **test.sh runs all examples** with pass/fail reporting
5. **README.md in each directory** explains the examples
6. **Consistent naming** pattern: `<subsystem>_<variant>_example.cpp`

## Documentation

- `examples/topics/README.md` - Complete guide to topic examples
- Inline comments in each example explaining key concepts
- Error handling patterns demonstrated
- Resource cleanup patterns shown

## Testing Verified

All examples tested and confirmed working:
- ✅ Compiles without errors
- ✅ Runs without crashes
- ✅ Properly cleans up resources
- ✅ Returns exit code 0 on success
- ✅ Compatible with test.sh automation

## Key Improvements

1. **Proper Organization**: Follows existing project structure exactly
2. **Build System Integration**: Works with existing build.sh and CMake setup
3. **Test Suite Integration**: Integrated into test.sh with pass/fail reporting
4. **Correct API Usage**: Uses actual exported FFI functions
5. **Comprehensive Examples**: Shows real-world integration patterns
6. **Documentation**: Complete README and inline comments
7. **Platform Support**: Linux event-driven mode with fallback for other platforms
8. **Error Handling**: Proper error checking and cleanup patterns

## Summary

The topic examples are now:
- ✅ Properly organized following project structure
- ✅ Integrated with existing build system
- ✅ Using correct API function names
- ✅ Comprehensive (basic → async → integration)
- ✅ Fully tested and working
- ✅ Well documented

No more "dead code in a cpp folder" - everything is properly integrated and follows the established patterns from `buffers/` and `region_manager/` examples.
