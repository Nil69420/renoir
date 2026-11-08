"""Pytest suite for the renoir-python extension module.

Covers the same core loop as tests/ffi_basic_tests.rs and
wrapper_drift_guard.cpp: create manager, register topic, publish,
subscribe/read-next, plus binding-layer error cases (type conversion and
exception mapping are what these tests prove).
"""

import time

import pytest

import renoir_python as renoir


def make_manager():
    mgr = renoir.TopicManager()
    return mgr


def test_create_topic_and_round_trip():
    mgr = make_manager()
    topic_id = mgr.create_topic(
        "round_trip", pattern="spsc", ring_capacity=64, max_payload_size=1024
    )
    assert topic_id > 0

    pub = mgr.create_publisher("round_trip")
    payload = b"hello from pytest"
    pub.publish(payload)

    sub = mgr.create_subscriber("round_trip")
    msg = sub.read_next(timeout_ms=1000)
    assert msg == payload


def test_multiple_messages_in_order():
    mgr = make_manager()
    mgr.create_topic("ordered", pattern="spsc", ring_capacity=64, max_payload_size=1024)
    pub = mgr.create_publisher("ordered")
    sub = mgr.create_subscriber("ordered")

    for i in range(5):
        pub.publish(b"msg-%d" % i)

    for i in range(5):
        msg = sub.read_next(timeout_ms=1000)
        assert msg == b"msg-%d" % i


def test_mpmc_multiple_publishers():
    mgr = make_manager()
    mgr.create_topic("mpmc_topic", pattern="mpmc", ring_capacity=256, max_payload_size=1024)
    pub1 = mgr.create_publisher("mpmc_topic")
    pub2 = mgr.create_publisher("mpmc_topic")
    sub = mgr.create_subscriber("mpmc_topic")

    pub1.publish(b"from-pub1")
    pub2.publish(b"from-pub2")

    received = []
    for _ in range(2):
        msg = sub.read_next(timeout_ms=1000)
        assert msg is not None
        received.append(msg)
    assert sorted(received) == [b"from-pub1", b"from-pub2"]


def test_read_next_timeout_returns_none():
    mgr = make_manager()
    mgr.create_topic("empty_topic", pattern="spsc", ring_capacity=64, max_payload_size=1024)
    sub = mgr.create_subscriber("empty_topic")
    assert sub.read_next(timeout_ms=200) is None


def test_invalid_pattern_raises():
    mgr = make_manager()
    with pytest.raises(renoir.RenoirError):
        mgr.create_topic("bad", pattern="bogus", ring_capacity=64, max_payload_size=1024)


def test_unknown_topic_publisher_raises():
    mgr = make_manager()
    with pytest.raises(renoir.RenoirError):
        mgr.create_publisher("does_not_exist")


def test_unknown_topic_subscriber_raises():
    mgr = make_manager()
    with pytest.raises(renoir.RenoirError):
        mgr.create_subscriber("does_not_exist")


def test_remove_topic():
    mgr = make_manager()
    mgr.create_topic("removable", pattern="spsc", ring_capacity=64, max_payload_size=1024)
    pub = mgr.create_publisher("removable")
    sub = mgr.create_subscriber("removable")
    pub.publish(b"x")
    assert sub.read_next(timeout_ms=1000) == b"x"
    del pub
    del sub
    mgr.remove_topic("removable")


def test_empty_payload_round_trip():
    mgr = make_manager()
    mgr.create_topic("empty_payload", pattern="spsc", ring_capacity=64, max_payload_size=1024)
    pub = mgr.create_publisher("empty_payload")
    sub = mgr.create_subscriber("empty_payload")
    pub.publish(b"")
    msg = sub.read_next(timeout_ms=1000)
    assert msg == b""


def test_large_payload_round_trip():
    mgr = make_manager()
    mgr.create_topic("large_payload", pattern="spsc", ring_capacity=256, max_payload_size=8192)
    pub = mgr.create_publisher("large_payload")
    sub = mgr.create_subscriber("large_payload")
    payload = bytes(range(256)) * 16  # 4 KiB
    pub.publish(payload)
    msg = sub.read_next(timeout_ms=1000)
    assert msg == payload


def test_many_messages_ring_full_behavior():
    mgr = make_manager()
    # Small ring so publish can hit "full" — publish must not corrupt state
    mgr.create_topic("tiny_ring", pattern="spsc", ring_capacity=4, max_payload_size=1024)
    pub = mgr.create_publisher("tiny_ring")
    sub = mgr.create_subscriber("tiny_ring")

    sent = 0
    for i in range(100):
        try:
            pub.publish(b"m-%d" % i)
            sent += 1
        except renoir.RenoirError:
            break

    received = 0
    for _ in range(sent):
        msg = sub.read_next(timeout_ms=1000)
        if msg is None:
            break
        received += 1

    assert received == sent


def test_lean_channel_round_trip(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean", 16, 64)
    sub = renoir.LeanChannel("py_lean", 16, 64, True)
    assert sub.msg_size == 16
    assert sub.try_read() is None
    for i in range(10):
        pub.publish(bytes([i]) * 16)
    assert sub.pending() == 10
    for i in range(10):
        assert sub.try_read() == bytes([i]) * 16
    assert sub.try_read() is None
    assert sub.lost() == 0


def test_lean_channel_wait_and_notify(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean_wait", 16, 64)
    sub = renoir.LeanChannel("py_lean_wait", 16, 64)
    assert sub.wait(10, 20) is False
    pub.publish_notify(b"x" * 16)
    assert sub.wait(0, 1000) is True
    assert sub.try_read() == b"x" * 16


def test_lean_channel_multi_producer(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    a = renoir.LeanChannel("py_lean_mp", 16, 64, False, True)
    b = renoir.LeanChannel("py_lean_mp", 16, 64, True, True)
    a.publish(b"a" * 16)
    b.publish(b"b" * 16)
    assert b.try_read() == b"a" * 16
    assert b.try_read() == b"b" * 16
    with pytest.raises(renoir.RenoirError):
        renoir.LeanChannel("py_lean_mp", 16, 64)


def test_lean_channel_gated_is_lossless(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean_gate", 16, 4, False, False, True, False)
    sub = renoir.LeanChannel("py_lean_gate", 16, 4, True, False, True, True)
    for _ in range(4):
        pub.publish(b"z" * 16)
    with pytest.raises(renoir.RenoirError):
        pub.publish(b"z" * 16)
    assert sub.try_read() == b"z" * 16
    pub.publish(b"y" * 16)
    assert sub.lost() == 0


def test_lean_channel_batches(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean_batch", 16, 64)
    sub = renoir.LeanChannel("py_lean_batch", 16, 64, True)
    payload = b"".join(bytes([i]) * 16 for i in range(5))
    assert pub.publish_batch(payload) == 5
    assert sub.read_batch(3) == payload[: 3 * 16]
    assert sub.read_batch(8) == payload[3 * 16 :]
    assert sub.read_batch(8) == b""
    with pytest.raises(renoir.RenoirError):
        pub.publish_batch(b"x" * 17)


def test_lean_channel_rejects_bad_sizes(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    with pytest.raises(renoir.RenoirError):
        renoir.LeanChannel("py_lean_bad", 0, 8)
    pub = renoir.LeanChannel("py_lean_bad2", 16, 8)
    with pytest.raises(renoir.RenoirError):
        pub.publish(b"short")


def test_rpc_round_trip_and_timeout(tmp_path, monkeypatch):
    import threading

    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    server = renoir.RpcServer("py_rpc", 32)
    client = renoir.RpcClient("py_rpc", 32)
    stop = threading.Event()

    def serve():
        while not stop.is_set():
            req = server.poll()
            if req is None:
                server.wait(5)
                continue
            client_id, sequence, payload = req
            server.reply(client_id, sequence, payload[::-1])

    thread = threading.Thread(target=serve)
    thread.start()
    try:
        assert client.call(b"hello", 2000) == b"olleh"
        assert client.call(b"abc", 2000) == b"cba"
    finally:
        stop.set()
        thread.join()
    lonely = renoir.RpcClient("py_rpc_nobody", 32)
    with pytest.raises(renoir.RenoirError):
        lonely.call(b"x", 30)


def test_lean_deadline(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean_dl", 16, 8)
    sub = renoir.LeanChannel("py_lean_dl", 16, 8)
    sub.set_deadline(20)
    assert sub.wait_deadline(10) is False
    assert sub.deadline_misses() == 1
    pub.publish(b"d" * 16)
    assert sub.wait_deadline(10) is True


def test_lean_memory_required():
    assert renoir.lean_memory_required(48, 64) == 1728 + 64 * 64
    assert renoir.lean_memory_required(100, 100) == 1728 + 64 * 128
    with pytest.raises(renoir.RenoirError):
        renoir.lean_memory_required(0, 64)


def test_lean_wait_adaptive(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    pub = renoir.LeanChannel("py_lean_adapt", 16, 8)
    sub = renoir.LeanChannel("py_lean_adapt", 16, 8)
    assert sub.wait_adaptive(10) is False
    pub.publish_notify(b"a" * 16)
    assert sub.wait_adaptive(1000) is True


def test_discover_lists_rings_and_sweep_keeps_open_ones(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    name = "py_discover_%d" % time.monotonic_ns()
    ch = renoir.LeanChannel(name, 24, 32)
    ch.publish(b"x" * 24)
    rings = {r["name"]: r for r in renoir.discover()}
    assert name in rings
    ring = rings[name]
    assert ring["kind"] == "lean"
    assert ring["msg_size"] == 24
    assert ring["slot_count"] == 32
    assert ring["published"] == 1
    assert ring["live_publishers"] >= 1
    assert len(ring["mapped_by"]) >= 1
    renoir.sweep(dry_run=True, ignore_unreadable=True)
    assert name in {r["name"] for r in renoir.discover()}
    assert ch.live_publishers() >= 1


def test_frame_channel_dynamic_sizes_and_back_pressure(tmp_path, monkeypatch):
    monkeypatch.setenv("RENOIR_SHM_DIR", str(tmp_path))
    name = "py_frame_%d" % time.monotonic_ns()
    pub = renoir.FrameChannel(name, 100_000, 4)
    sub = renoir.FrameChannel(name, 100_000, 4, consumer=True)
    assert sub.max_frame == 100_000
    assert sub.read() is None
    sizes = [1, 700, 100_000, 31]
    for i, size in enumerate(sizes):
        pub.publish(bytes([i]) * size)
    with pytest.raises(renoir.RenoirError):
        pub.publish(b"x" * 10)
    for i, size in enumerate(sizes):
        frame = sub.read()
        assert frame == bytes([i]) * size
    assert sub.read() is None
    pub.publish(b"after")
    assert sub.wait(spin_iterations=10, timeout_ms=100) is True
    assert sub.read() == b"after"
    assert sub.consumer_id() is not None
    assert pub.consumer_id() is None
    with pytest.raises(renoir.RenoirError):
        pub.publish(b"")
