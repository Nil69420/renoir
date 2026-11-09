# Discovery and monitoring

Renoir needs no central daemon. Every ring is a file named `renoir_<name>` in the shared-memory directory (`RENOIR_SHM_DIR`, otherwise `/dev/shm`), and every ring header describes itself, so discovery is a read-only scan of that directory.

What the scan reports for each ring: name, kind (lean, broadcast, spsc, mpmc), file size, and the processes that currently map the file. For lean rings it also reports message size, slot count, mode, messages published, live publisher processes, and registered gated consumers.

Nothing in the data path registers with anything, so discovery costs nothing at publish time and cannot go stale: a publisher that is killed disappears from the live count as soon as the operating system reports its process gone.

## From Rust, C, C++ and Python

- Rust: `renoir::discovery::scan()` returns `Vec<RingInfo>`; `sweep(SweepOptions)` removes unused ring files.
- C: `renoir_discover(out, capacity, &total)` fills up to `capacity` entries and always sets `total`; call once with capacity 0 to size the buffer. `renoir_discover_sweep(flags, &removed)` with `RENOIR_SWEEP_DRY_RUN` and `RENOIR_SWEEP_IGNORE_UNREADABLE`.
- C++: `renoir::discover()` and `renoir::sweep()` in `renoir_discovery.hpp`.
- Python: `renoir.discover()` and `renoir.sweep()`.

## Watching for changes

`DiscoveryWatcher` (Rust) and `renoir_discover_watch_open/fd/poll/close` (C) report ring `Appeared`, `Changed` and `Disappeared` events instead of making you rescan. The watcher puts an inotify watch on the shared-memory directory, so a ring file being created or deleted wakes `poll` immediately (it also exposes the inotify descriptor for `poll(2)` or an event loop). Changes inside a ring, such as a publisher or consumer registering, or a publisher process dying, are memory writes that raise no file event; the watcher finds them with a periodic rescan that reads only the ring headers (no `/proc` scan), at the interval you pass (a few tens of milliseconds is cheap). A ring file that exists but is not yet initialised is rescanned every 5 ms for up to two seconds, so appearance is reported within a few milliseconds of initialisation. A `Changed` event carries the previous and the current description, so the previous publisher and consumer counts are visible. `renoir-monitor events` prints the stream. Measured in the test suite: appearance and removal are reported in under 500 ms (typically within a few milliseconds), and a watcher with a 30 s interval is woken within a second by a new ring.

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
