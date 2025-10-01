# Renoir Examples

This directory contains comprehensive C++ examples demonstrating how to use the Renoir shared memory library through its C FFI (Foreign Function Interface).

## Overview

Renoir is a high-performance shared memory library designed for embedded systems and real-time applications. These examples showcase the main features using the correct working API.

## Prerequisites

- **Rust Toolchain**: Required to build the Renoir library
- **CMake**: Version 3.16 or higher
- **Ninja Build System**: For fast parallel builds
- **C++ Compiler**: Supporting C++17 (GCC 7+ or Clang 5+)
- **Linux System**: Examples use Linux-specific features (MemFd)

## Building

### Quick Start

```bash
# Build everything (Rust library + C++ examples)
./build.sh build-all

# Or build step by step
./build.sh build-rust    # Build Renoir library first
./build.sh build-examples # Build C++ examples
```

### Manual Build

```bash
# 1. Build Rust library
cargo build --release

# 2. Build examples
cd examples
mkdir -p build
cd build
cmake -G Ninja ..
ninja
```

## Examples

### 1. Region Manager Example

**Location**: `region_manager/region_manager_example.cpp`

Demonstrates basic shared memory region operations:

```bash
# Run the example
./build/region_manager_example

# Or using ninja target
cd build && ninja run_region_manager_example
```

**Features shown**:
- Creating memory managers
- Opening and creating shared memory regions
- Region statistics and monitoring
- Proper cleanup and resource management

### 2. Buffer Pool Example

**Location**: `buffers/buffer_example.cpp`

Shows advanced buffer pool management:

```bash
# Run the example
./build/buffer_example

# Or using ninja target
cd build && ninja run_buffer_example
```

**Features shown**:
- Creating and configuring buffer pools
- Allocating and returning buffers
- Multi-threaded stress testing
- Pool statistics and health monitoring

### 3. Ring Buffer Example

**Location**: `buffers/ring_buffer_example.cpp`

Demonstrates ring buffer operations:

```bash
# Run the example
./build/ring_buffer_example

# Or using ninja target
cd build && ninja run_ring_buffer_example
```

**Features shown**:
- Creating ring buffers
- Ring buffer state management
- Statistics monitoring

### 4. Control Region Example

**Location**: `buffers/control_region_example.cpp`

Shows metadata and registry management:

```bash
# Run the example
./build/control_region_example

# Or using ninja target
cd build && ninja run_control_region_example
```

**Features shown**:
- Creating control regions for metadata storage
- Registering buffer pools with control regions
- Listing and managing region registry
- Cleanup operations

## API Usage

### Error Handling

All functions return `RenoirErrorCode`. Always check return values:

```cpp
RenoirErrorCode error = renoir_buffer_get(pool, &buffer);
if (error != Success) {
    // Handle error
    return error;
}
```

### Manager Operations

```cpp
// Create manager (no config needed)
RenoirManagerHandle manager = renoir_manager_create();

// Destroy manager
RenoirErrorCode error = renoir_manager_destroy(manager);
```

### Region Operations

```cpp
// Region configuration
RenoirRegionConfig config = {
    "region_name",    // name
    1024 * 1024,      // size
    1,                // backing_type (MemFd)
    nullptr,          // file_path
    true,             // create
    0644              // permissions
};

// Create region
RenoirRegionHandle region;
RenoirErrorCode error = renoir_region_create(manager, &config, &region);
```

### Buffer Pool Operations

```cpp
// Buffer pool configuration
RenoirBufferPoolConfig pool_config = {
    "pool_name",      // name
    4096,             // buffer_size
    10,               // initial_count
    50,               // max_count
    64,               // alignment
    true,             // pre_allocate
    1000              // allocation_timeout_ms
};

// Create buffer pool
RenoirBufferPoolHandle pool;
RenoirErrorCode error = renoir_buffer_pool_create(manager, "region_name", &pool_config, &pool);

// Get/return buffers
RenoirBufferHandle buffer;
error = renoir_buffer_get(pool, &buffer);
error = renoir_buffer_return(pool, buffer);
```

## Troubleshooting

### Common Issues

1. **Compilation Errors**: Ensure you're using the correct API signatures
2. **Linking Errors**: Make sure Renoir library is built first
3. **Runtime Errors**: Check permissions for shared memory operations

### API Notes

- Error codes use `Success`, not `RenoirErrorCode_Success`
- `renoir_manager_create()` takes no parameters
- `renoir_region_size()` returns size directly
- Always call destroy/close functions in reverse creation order

## License

The Renoir library and examples are licensed under the MIT License.