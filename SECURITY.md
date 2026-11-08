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
- The lean ring allows a single publisher per ring. A second writer is a protocol violation
  and can only corrupt message ordering and contents, not memory safety of readers.
- Dependencies are checked with `cargo audit` and `cargo deny` in CI. One advisory
  (bincode, unmaintained, no known vulnerability) is explicitly allowed in `deny.toml`; the
  decoder is size-bounded.

Report vulnerabilities privately to the maintainer.
