# Frame rings: dynamic-size, zero-copy, owned

A frame ring carries messages of any length from 1 byte up to a maximum you choose at open time (up to 64 MiB), written in place by the producer and read in place by consumers, with no copy on either side.

```
let mut producer = FrameChannel::open("camera", 921_600, 8, false, false, false)?;
let mut consumer = FrameChannel::open("camera", 921_600, 8, false, true, false)?;

let mut loan = producer.loan(640 * 480 * 3)?;   // write in place
fill_image(loan.bytes_mut());
loan.commit()?;                                  // publish

if let Some(view) = consumer.view()? {           // read in place
    process(view.bytes());
}                                                // dropping the view releases the slot
```

## Ownership

A view is owned: the consumer does not advance its gate position until the view is dropped, so the producer cannot reuse that slot while it is held. This is the difference from the broadcast view (`renoir_subscribe_view`), where the producer may overwrite the frame and the consumer must validate afterwards. Frame rings are always gated and lossless; a full ring makes `loan` return `BufferFull` instead of overwriting.

- A held view blocks the producer once the ring is full. A frame ring with `N` slots lets a consumer hold up to one frame while the producer keeps going for `N - 1` more.
- A consumer that is alive is never evicted by a timer, however long it holds a frame.
- A consumer whose process has died (checked by process id and start time) is reclaimed after the gate timeout, so a crashed holder cannot wedge the ring.
- A producer that dies between `loan` and `commit` leaves nothing visible: single-producer rings simply reuse the slot, multi-producer rings hold the hole until the liveness check skips it.
- Dropping a loan without committing aborts it.

## Dynamic length

Each frame carries its own length. `loan(len)` takes any length from 1 to the ring's maximum frame size; the slot is sized for the maximum. Memory is `3264 B header + slots x (round_up_64(16 + max_frame))`; use `LeanRing::required_frame_memory` or size it from the maximum frame you expect.

## Lifespan

Each frame is stamped with a monotonic clock when it is committed. `view_fresh(max_age)` skips (and releases) frames older than `max_age` and returns the first fresh one; skipped frames are counted by `stale_dropped()`. This keeps a slow consumer from processing a backlog of stale camera images.

## Modes and features it shares with the lean ring

Single or multi producer, consumer registration, consumer dependencies (`set_upstream`: a downstream stage only sees frames the upstream has released), blocking wait with `commit_notify`, discovery (`renoir-monitor` lists frame rings; their message size shows the slot payload including the 8 byte frame header), and liveness-based recovery.

## Interfaces

- Rust: `FrameChannel`, `FrameLoan`, `FrameView`.
- C: `renoir_frame_open`, `renoir_frame_loan`, `renoir_frame_commit`, `renoir_frame_commit_notify`, `renoir_frame_abort`, `renoir_frame_view`, `renoir_frame_release`, `renoir_frame_view_intact`, `renoir_frame_wait`, `renoir_frame_consumer_id`, `renoir_frame_set_upstream`. One loan and one view can be outstanding per handle.
- C++: `renoir_frame.hpp` with RAII `FrameChannel::Loan` (aborts if not committed) and `FrameChannel::View` (releases on destruction).
- Python: `renoir.FrameChannel` with `publish(bytes)` and `read()` (copying; zero-copy is available from Rust, C and C++).

## Limits

- Liveness uses process ids: processes sharing a frame ring must share a pid namespace, otherwise a live holder could be judged dead and its frame reused.
- Frame rings cannot be opened as lean rings and the plain lean read calls must not be used on them.
- At most 32 registered consumers and 16 registered publishers per ring.
