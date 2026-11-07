# Python Binding

The Python bindings live in the `renoir-python` crate (a separate crate
that path-depends on the core `renoir` crate — the core crate is not
restructured for it). The module is built with maturin and PyO3.

## Install

```sh
cd renoir-python
maturin develop        # installs into the active virtualenv
# or
maturin build          # produces a wheel under target/wheels/
pip install target/wheels/renoir_python-*.whl
```

## Usage

```python
import renoir_python as renoir

mgr = renoir.TopicManager()
mgr.create_topic("my_topic", pattern="spsc", ring_capacity=64,
                 max_payload_size=1024)
pub = mgr.create_publisher("my_topic")
pub.publish(b"hello")
sub = mgr.create_subscriber("my_topic")
msg = sub.read_next(timeout_ms=1000)   # -> b'hello', or None on timeout
```

## API

- `TopicManager.create_topic(name, pattern, ring_capacity,
  max_payload_size)` — `pattern` is one of `"spsc"`, `"spmc"`, `"mpsc"`,
  `"mpmc"`
- `TopicManager.create_publisher(name)` / `create_subscriber(name)`
- `Publisher.publish(bytes)` — raises `renoir.RenoirError` on failure
- `Subscriber.read_next(timeout_ms)` — returns bytes, or `None` on
  timeout/empty

Errors from the Rust layer are surfaced as `renoir.RenoirError`.

## Tests

The pytest suite in `renoir-python/tests/` covers the same core loop as
the C API tests, including error cases (invalid pattern, unknown topic,
timeout).

## Limitations

Descriptor-based (shared-pool) payloads are not yet exposed in the Python
API; `read_next` raises `RenoirError` if it receives one.
