# Migrating from iceoryx2

iceoryx2 is Renoir's closest architectural peer: a Rust-core, lock-free,
zero-copy shared-memory pub/sub with no central broker.

## What you gain

- **Variable-size payloads.** iceoryx2's own ROS2 integration
  (`rmw_iceoryx`) is documented as supporting only fixed-size message
  types. Renoir's inline ring storage and shared buffer pools handle
  variable sizes.
- **Python bindings** out of the box (iceoryx2 has C bindings; Python
  requires a third-party wrapper).
- **A smaller surface.** One crate, one C header, one CLI. iceoryx2's
  service model (nodes, services, ports, wait sets) is more machinery to
  learn.
- **Crash-safe region reuse** via `flock()` liveness detection: an
  orphaned region from a killed process is automatically reinitialized on
  next attach.

## What you give up

- **Latency.** In our same-hardware benchmark (256-byte payload, 100k
  round trips), iceoryx2 0.9.3 beat Renoir 0.1.0 by roughly 1.8x on mean
  latency and throughput. The gap is partly architectural: iceoryx2 loans
  zero-copy samples in both directions, while Renoir's inline ring path
  copies through the ring (and computes a CRC32 per message). See
  [Performance](../performance.md) for the full numbers and methodology.
- **Maturity.** iceoryx2 is an Eclipse project with years of real-world
  deployment; Renoir is pre-1.0.
- **Tooling.** iceoryx2 ships an introspection client; Renoir has a
  minimal `renoir-cli topics --watch` (no GUI).
- **Ecosystem.** `rmw_iceoryx` gives iceoryx2 a supported ROS2 on-ramp;
  Renoir has no ROS2 middleware plugin yet.

## Migration path

The core loop maps directly:

| iceoryx2 | Renoir |
|---|---|
| `NodeBuilder::new().create()` | `renoir_topic_manager_create()` |
| `service_builder(...).open_or_create()` | `renoir_topic_register(mgr, name, &opts, &tid)` |
| `service.publisher_builder().create()` | `renoir_publisher_create(mgr, name, ...)` |
| `publisher.send_copy(data)` | `renoir_publish(pub, data, len, ...)` |
| `subscriber.receive()` | `renoir_subscribe_read_next(sub, &msg, timeout_ms)` |
