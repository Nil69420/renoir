# Getting Started

This tutorial takes you from a clean clone to a working publish/subscribe
round trip. The exact commands below were run against this repository's
current state and work literally.

## 1. Build the library

```sh
cargo build --release --features c-api
```

This produces `target/release/librenoir.so` (and `.a`). The generated C
header lands at both `target/include/renoir.h` and the checked-in
`include/renoir.h` (they are byte-identical; `build.rs` regenerates
`include/renoir.h` from `cbindgen.toml` on every build).

## 2. First round trip in C

Save this as `first.c`:

```c
#include <renoir.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    RenoirTopicManagerHandle mgr = renoir_topic_manager_create();
    if (!mgr) { fprintf(stderr, "failed to create manager\n"); return 1; }

    RenoirTopicOptions opts = {0};
    opts.pattern = 0;                 /* SPSC */
    opts.ring_capacity = 16;
    opts.max_payload_size = 4096;
    opts.enable_notifications = true;

    RenoirTopicId tid;
    if (renoir_topic_register(mgr, "/hello", &opts, &tid) != RenoirErrorCode_Success) {
        fprintf(stderr, "failed to register topic\n"); return 1;
    }

    RenoirPublisherHandle pub = NULL;
    renoir_publisher_create(mgr, "/hello", NULL, &pub);
    RenoirSubscriberHandle sub = NULL;
    renoir_subscriber_create(mgr, "/hello", NULL, &sub);

    const char *msg = "hello from renoir";
    RenoirSequenceNumber seq = 0;
    renoir_publish(pub, (const uint8_t *)msg, strlen(msg), NULL, &seq);

    RenoirReceivedMessage recv = {0};
    if (renoir_subscribe_read_next(sub, &recv, 1000) == RenoirErrorCode_Success) {
        printf("received %zu bytes: %.*s\n", recv.payload_len,
               (int)recv.payload_len, (const char *)recv.payload_ptr);
        renoir_message_release(recv.handle);
    }

    renoir_publisher_destroy(pub);
    renoir_subscriber_destroy(sub);
    renoir_topic_manager_destroy(mgr);
    return 0;
}
```

Compile and run:

```sh
cc first.c -Iinclude -Ltarget/release -lrenoir -o first
LD_LIBRARY_PATH=target/release ./first
```

Expected output: `received 17 bytes: hello from renoir`

## 3. Cross-process pub/sub

Topics are file-backed, so a second process can attach to the same region
by name and exchange messages. See
[`tests/cross_process_tests.rs`](../tests/cross_process_tests.rs) for a
full parent/child round trip.

## 4. Inspect live topics

While a publisher runs, from another terminal:

```sh
cargo run --release --features cli -- topics --watch
```

Shows every live topic ring in shared memory (capacity, pending bytes,
utilization, latency) refreshing every second.

## 5. Python

```sh
cd renoir-python
maturin develop            # or: pip install <wheel from maturin build>
```

```python
import renoir_python as renoir

mgr = renoir.TopicManager()
mgr.create_topic("my_topic", pattern="spsc", ring_capacity=64, max_payload_size=1024)
pub = mgr.create_publisher("my_topic")
pub.publish(b"hello")
sub = mgr.create_subscriber("my_topic")
print(sub.read_next(timeout_ms=1000))   # -> b'hello'
```

See [Python](bindings/python.md) for details.
