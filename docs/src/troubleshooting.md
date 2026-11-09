# Troubleshooting

## `Existing lean ring has a different geometry` / `different producer mode`
The ring file already exists with another message size, slot count or producer mode. Every process must open a name with the same `msg_size`, `slot_count` and flags. Remove the stale file in `RENOIR_SHM_DIR` (default `/dev/shm`) after stopping all users, or choose a new name.

## A reader sees `lost` growing
It was lapped by the publisher. Raise the slot count (see Sizing), read faster, call `skip_to_latest` when only the newest value matters, or open the ring gated and register the reader as a consumer so the publisher waits instead.

## `BufferFull` from a gated ring
A registered consumer has not read for the ring's whole depth. Either the consumer is too slow (raise depth or speed it up) or it is stuck. A consumer that makes no progress for 100 ms is evicted automatically and the publisher continues; the evicted consumer's `lost` counter shows what it missed.

## `All lean consumer slots are in use`
A gated ring supports 32 registered consumers. Unregister or close consumers that are gone; a crashed consumer's entry is reclaimed after eviction.

## RPC `call` returns `BufferEmpty`
The timeout elapsed with no response: the server is not running, is busy, or uses a different `max_payload` or `request_slots` (those mismatches fail at open). Check that a server is running for the service: `renoir-monitor list` shows the `<service>_req` rings and how many live consumers each has.

## Latency is worse than the published numbers
- Pin both threads to different physical cores (not hyperthread siblings) and keep the CPU governor on `performance`.
- Interrupts and the scheduler add microsecond tails; for bounded tails use `isolcpus`, `nohz_full` and IRQ affinity.
- A consumer that blocks in `wait` pays a futex wake-up (about 10 microseconds); spin with `try_read` if the budget is 100 ns.
- Do not run other CPU-heavy processes on the pinned cores; one stray spinning process added tens of nanoseconds in our own measurements.
- Messages over 56 bytes use more than one cache line per slot and cost an extra line transfer.

## Python or C++ cannot find `librenoir.so.0`
The library has the SONAME `librenoir.so.0`. Install it with its symlinks (the Debian package and the Jetson tarball do) or set `LD_LIBRARY_PATH` to the directory that contains `librenoir.so.0`.

## No `/dev/shm` space
Rings live in RAM. Check `df -h /dev/shm`; set `RENOIR_SHM_DIR` to another tmpfs if needed.

## Opening a ring fails with "Not enough space ... shared-memory filesystem"
Rings reserve their whole size when they are created, so a full or small `/dev/shm` is reported at open. In Docker the default is 64 MB: start the container with `--shm-size=1g` (or whatever your rings need, see `docs/src/sizing.md`), or point `RENOIR_SHM_DIR` at a larger tmpfs. Rings are never sparse, so the memory is committed from the moment of creation.

## Opening a ring fails with "Existing region is smaller than the requested size"
Another process created the same ring with a smaller geometry. Message size, slot count and maximum frame size must match between all processes using a ring. The existing ring is left untouched.
