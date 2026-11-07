# Migrating from Zenoh

Zenoh is a routing + shared-memory transport (`zenoh-shm`) aimed at
edge/IoT and multi-host scenarios, with growing ROS2 adoption via
`rmw_zenoh`.

## What you gain

- **Same-host latency without routing overhead.** Zenoh's value is
  routing and its network protocol; if your data never leaves the host,
  that machinery is overhead Renoir doesn't have.
- **A simpler data plane.** One host, one region, one ring — no session
  negotiation or scouting.

## What you give up

- **Multi-host networking.** Zenoh moves data across machines; Renoir is
  host-local by design. The intended pattern is a Zenoh (or DDS) bridge
  for anything that leaves the host — that bridge is not built yet.
- **Ecosystem.** `rmw_zenoh` gives Zenoh a ROS2 on-ramp; Renoir has no
  ROS2 middleware plugin yet.
- **Maturity.** Zenoh is widely deployed (Eclipse project); Renoir is
  pre-1.0.

## Migration path

For same-host, latency-critical pub/sub, Zenoh's publisher/subscriber
calls map directly onto Renoir topics. For anything requiring
multi-host delivery, keep Zenoh and use a gateway between the two — Renoir
deliberately does not reimplement networking.
