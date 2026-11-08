# Security model

Renoir shares memory between processes that already trust each other at the operating-system
level. Access control is the file permission on the region file (default 0644 under
`/dev/shm`, or `RENOIR_SHM_DIR`). Run publisher and subscribers under the same user or group
and tighten permissions where needed.

- Every ring header read from shared memory is validated (magic, geometry, bounds) before use.
  A hostile writer can corrupt message contents and cause lost messages, but readers do not
  read out of bounds, hang, or crash on corrupt headers or slots. This is covered by tests
  and the `*_corrupt_ring` fuzz targets.
- Message payloads are untrusted bytes. Validate them before use.
- A malicious process that can write the region file can disrupt delivery. Renoir does not
  defend against a process with write access to the region.
- A lean ring allows a single publisher unless it was opened in multi-producer mode, which
  is recorded in the ring header and checked on open. A second writer on a single-producer
  ring is a protocol violation and can only corrupt message ordering and contents, not memory
  safety of readers. Gated-consumer positions and request-response headers come from shared
  memory and are bounds-checked before use; a hostile process can stall or evict consumers
  and spoof request client ids (the server caps cached response rings at 64) but cannot make
  readers read out of bounds.
- Frame rings hand out owned in-place views: a consumer holding a view keeps the producer from reusing that slot, and a holder is only evicted when its process is dead (process id and start time). This assumes every process sharing a ring shares a pid namespace; across namespaces a live holder could be judged dead and its frame reused. Frame lengths are read from shared memory and bounds-checked before use (fuzzed by `frame_corrupt_ring`).
- Liveness decisions (multi-producer hole skipping, slot takeover, gated eviction, RPC failover, `renoir-monitor sweep`) use process ids read from shared memory. A hostile writer to the region can forge them to keep a dead entry alive or to get a live entry reclaimed; this is the same trust boundary as write access to the region. `sweep` deletes a ring file only when no readable process maps it and, by default, refuses when a process whose mappings it cannot read could hold it.
- The in-place frame view (`renoir_subscribe_view`) hands out a pointer into memory the
  producer may rewrite. Treat the bytes as untrusted until `renoir_subscribe_view_release`
  reports they were not overwritten, and never use them as lengths or offsets before then.
- Dependencies are checked with `cargo audit` and `cargo deny` in CI. One advisory
  (bincode, unmaintained, no known vulnerability) is explicitly allowed in `deny.toml`; the
  decoder is size-bounded.

Report vulnerabilities privately to the maintainer.
