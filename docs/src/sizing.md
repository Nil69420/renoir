# Sizing and memory

## Lean ring

A lean ring occupies one region, file-backed under `RENOIR_SHM_DIR` (default `/dev/shm`):

```
bytes = 704 + slots x slot_bytes
slot_bytes = 64 x ceil((msg_size + 8) / 64)        (msg_size 1..=504)
```

`slots` is rounded down to a power of two when the ring is created from a byte budget, and must be at least 2. The header is 192 bytes of control data plus a 512-byte table for up to 8 gated consumers.

| msg_size | slot_bytes | 1024 slots |
|---|---|---|
| 1..=56 | 64 | 64 KiB + 704 B |
| 57..=120 | 128 | 128 KiB + 704 B |
| 121..=184 | 192 | 192 KiB + 704 B |
| 441..=504 | 512 | 512 KiB + 704 B |

Choosing the slot count: a lossy ring needs enough slots that a reader pausing for its worst-case scheduling delay is not lapped, `slots >= rate x worst_pause`. An IMU at 1 kHz with a 20 ms worst pause needs 20 slots, so 32. A gated ring needs the same figure for a consumer to avoid making the publisher see `BufferFull`.

## Broadcast ring (frames and larger messages)

```
bytes = 192 + slots x stride
stride = ceil((PAYLOAD_OFFSET + round_up(payload, 8)) / SLOT_ALIGN) x SLOT_ALIGN, PAYLOAD_OFFSET = 128 (16-byte slot header plus the 112-byte message area)
```

Use `BroadcastTopicRing::slot_stride(payload)` and `required_memory_size(ring_bytes, payload)`. A 921,600-byte camera frame with 8 slots needs about 7.4 MB. Frames are consumed with `renoir_subscribe_view` (no copy, must be validated with `renoir_subscribe_view_release`) or `renoir_subscribe_read_into` (one copy).

## Request-response

Request ring: lean multi-producer ring with `16 + max_payload` byte messages and `request_slots` slots. Each client adds a response ring of 16 slots of the same message size. With `max_payload` 64 (80-byte messages, 128-byte slots) and 64 request slots, the request ring is 704 + 64 x 128 = 8896 bytes and each client costs 704 + 16 x 128 = 2752 bytes.
