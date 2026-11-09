# Discovery and monitoring

Renoir needs no central daemon. Every ring is a file named `renoir_<name>` in the shared-memory directory (`RENOIR_SHM_DIR`, otherwise `/dev/shm`), and every ring header describes itself, so discovery is a read-only scan of that directory.

What the scan reports for each ring: name, kind (lean, broadcast, spsc, mpmc), file size, and the processes that currently map the file. For lean rings it also reports message size, slot count, mode, messages published, live publisher processes, and registered gated consumers.

Nothing in the data path registers with anything, so discovery costs nothing at publish time and cannot go stale: a publisher that is killed disappears from the live count as soon as the operating system reports its process gone.

## From Rust, C, C++ and Python

- Rust: `renoir::discovery::scan()` returns `Vec<RingInfo>`; `sweep(SweepOptions)` removes unused ring files.
- C: `renoir_discover(out, capacity, &total)` fills up to `capacity` entries and always sets `total`; call once with capacity 0 to size the buffer. `renoir_discover_sweep(flags, &removed)` with `RENOIR_SWEEP_DRY_RUN` and `RENOIR_SWEEP_IGNORE_UNREADABLE`.
- C++: `renoir::discover()` and `renoir::sweep()` in `renoir_discovery.hpp`.
- Python: `renoir.discover()` and `renoir.sweep()`.

## The renoir-monitor tool

```
renoir-monitor list            one-shot table
renoir-monitor watch 0.5       refresh every 0.5 s, with messages per second
renoir-monitor sweep --dry-run show which ring files no process has mapped
renoir-monitor sweep           delete them
```

## Cleaning up after crashes

A ring file stays on disk after every process using it has died. `sweep` deletes a ring file only when no process maps it. Reading another process's mappings needs permission: if a process whose maps cannot be read could still hold the ring (same user, or the file is group or world writable), sweep keeps the file and reports it as unverifiable. Pass `--ignore-unreadable` (`RENOIR_SWEEP_IGNORE_UNREADABLE`) when you know those processes do not use Renoir.

## Limits

- Liveness is judged by process id and start time. Processes in separate pid namespaces that share the directory cannot see each other.
- Broadcast, SPSC and MPMC rings are listed with kind and file size only.
- The counts are a snapshot read without synchronisation and can be momentarily inconsistent.
