use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use renoir::rpc::{RpcClient, RpcServer};
use renoir::topic::{MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{
    lean_slot_bytes, BroadcastTopicRing, LeanMode, LeanRead, LeanRing, ViewRead,
};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocs() -> usize {
    ALLOCS.load(Ordering::SeqCst)
}

fn lean_ring(mode: LeanMode, msg: usize) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 64;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring =
        unsafe { LeanRing::from_memory_mode(ptr, mem.len() * 8, ring_bytes, msg, mode).unwrap() };
    (Arc::new(ring), mem)
}

fn every_lean_mode_publishes_and_reads_without_allocating() {
    for mode in [
        LeanMode::SingleProducer,
        LeanMode::MultiProducer,
        LeanMode::SingleProducerGated,
        LeanMode::MultiProducerGated,
    ] {
        let (ring, _mem) = lean_ring(mode, 48);
        let mut publisher = ring.publisher();
        let mut plain = ring.cursor_latest();
        let mut gated = if mode.is_gated() {
            Some(ring.register_consumer(false).unwrap())
        } else {
            None
        };
        let payload = [9u8; 48];
        let batch = [7u8; 48 * 4];
        let mut out = [0u8; 48];
        let mut batch_out = [0u8; 48 * 4];
        let before = allocs();
        for _ in 0..20_000 {
            publisher.publish(&payload).unwrap();
            publisher.publish_notify(&payload).unwrap();
            assert_eq!(publisher.publish_batch(&batch).unwrap(), 4);
            let _ = ring.wait_readable(&plain, 0, Some(Duration::ZERO));
            match gated.as_mut() {
                Some(g) => {
                    for _ in 0..6 {
                        assert!(matches!(
                            ring.try_read_gated(g, &mut out),
                            Ok(LeanRead::Message)
                        ));
                    }
                    let _ = ring.pending_gated(g);
                }
                None => {
                    for _ in 0..2 {
                        assert!(matches!(
                            ring.try_read(&mut plain, &mut out),
                            Ok(LeanRead::Message)
                        ));
                    }
                    assert_eq!(ring.read_batch(&mut plain, &mut batch_out).unwrap(), 4);
                }
            }
        }
        assert_eq!(allocs() - before, 0, "allocation in mode {mode:?}");
    }
}

fn broadcast_reserve_commit_and_view_do_not_allocate() {
    let payload_cap = 4096;
    let ring_bytes = BroadcastTopicRing::slot_stride(payload_cap) * 16;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, payload_cap);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            mem.len() * 8,
            ring_bytes,
            payload_cap,
            Arc::new(TopicStats::default()),
        )
        .unwrap()
    };
    let mut cursor = ring.cursor_latest();
    let mut header = MessageHeader::new(1, 0, 1000);
    header.set_format_type(FORMAT_TYPE_INLINE);
    header.set_no_checksum();
    let before = allocs();
    for _ in 0..20_000 {
        let r = ring.reserve(1000).unwrap();
        unsafe { std::ptr::write_bytes(r.payload_ptr(), 3, 1000) };
        ring.commit(r, &header, 1000).unwrap();
        match ring.view(&mut cursor).unwrap() {
            ViewRead::View(v) => assert!(ring.release_view(&mut cursor, &v).unwrap()),
            ViewRead::Empty => panic!("expected a message"),
        }
    }
    assert_eq!(allocs() - before, 0);
}

fn rpc_call_and_reply_do_not_allocate_in_steady_state() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("RENOIR_SHM_DIR", dir.path());
    let service = format!("zero_alloc_rpc_{}", std::process::id());
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let server = {
        let stop = stop.clone();
        let service = service.clone();
        std::thread::spawn(move || {
            let mut server = RpcServer::open(&service, 32, 64).unwrap();
            ready_tx.send(()).unwrap();
            let mut buf = [0u8; 32];
            while !stop.load(Ordering::Relaxed) {
                match server.poll(&mut buf).unwrap() {
                    Some(req) => server
                        .reply(req.client_id, req.sequence, &buf[..req.len])
                        .unwrap(),
                    None => std::hint::spin_loop(),
                }
            }
        })
    };
    ready_rx.recv().unwrap();
    let mut client = RpcClient::open(&service, 32, 64).unwrap();
    let request = [5u8; 16];
    let mut response = [0u8; 32];
    for _ in 0..50 {
        client
            .call(&request, &mut response, Duration::from_secs(5))
            .unwrap();
    }
    let before = allocs();
    for _ in 0..5_000 {
        client
            .call(&request, &mut response, Duration::from_secs(5))
            .unwrap();
    }
    let after = allocs();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert_eq!(after - before, 0);
}

#[test]
fn hot_paths_do_not_allocate() {
    every_lean_mode_publishes_and_reads_without_allocating();
    broadcast_reserve_commit_and_view_do_not_allocate();
    if !cfg!(miri) {
        rpc_call_and_reply_do_not_allocate_in_steady_state();
    }
}
