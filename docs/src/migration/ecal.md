# Migrating from eCAL

eCAL (Continental) is a mature, automotive-pedigreed pub/sub +
client-server middleware with its own shared-memory layer.

## What you gain

- **Deployment simplicity.** eCAL is a distributed system (monitoring
  processes, separate daemons, multi-host capable). Renoir is a single
  shared library + optional CLI for one host.
- **A smaller attack surface.** Fewer daemons and services to run and
  secure.
- **Rust-native core** with C/C++/Python bindings, if your team prefers
  Rust infrastructure over C++ daemons.

## What you give up

- **Enterprise tooling.** eCAL ships recording/replay and a monitor GUI —
  the production-observability story Renoir does not have yet (only a
  minimal CLI table).
- **Multi-host support.** eCAL routes across machines; Renoir is
  host-local by design (bridge out via Zenoh/DDS instead).
- **Client-server pattern.** eCAL has pub/sub *and* client-server; Renoir
  currently has pub/sub only.
- **Provenance.** eCAL has years of automotive deployment; Renoir is
  pre-1.0.

## Migration path

For a same-host pub/sub workload, map eCAL topic pub/sub to Renoir topics
one-to-one (publish + subscribe calls). Anything relying on eCAL's
recording/replay or cross-host routing needs additional tooling or a
bridge.
