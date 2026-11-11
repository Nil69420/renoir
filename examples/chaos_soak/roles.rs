use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use renoir::rpc::{RpcClient, RpcServer};
use renoir::topic_rings::{LeanChannel, LeanQos, LeanRead};

pub const MSG: usize = 48;
const RPC_PAYLOAD: usize = 32;
const RPC_SLOTS: usize = 16;

pub fn qos_for(topic: &str, id: u64) -> LeanQos {
    match topic {
        "soak_be" => LeanQos::best_effort(256),
        "soak_rel" => LeanQos::reliable(64),
        _ => LeanQos::best_effort(128).exclusive(id.max(1)),
    }
}

fn encode(seq: u64, id: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    b[0..8].copy_from_slice(&seq.to_le_bytes());
    b[8..16].copy_from_slice(&id.to_le_bytes());
    for (i, chunk) in b[16..].chunks_exact_mut(8).enumerate() {
        let word = seq.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ id ^ i as u64;
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    b
}

fn word(b: &[u8], i: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[i * 8..i * 8 + 8]);
    u64::from_le_bytes(w)
}

fn verify(b: &[u8; MSG]) -> (u64, u64) {
    let (seq, id) = (word(b, 0), word(b, 1));
    for i in 0..4 {
        let want = seq.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ id ^ i as u64;
        assert_eq!(word(b, 2 + i), want, "torn or corrupt message");
    }
    (seq, id)
}

fn report(role: &str, count: u64) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "P {role} {count}");
    let _ = out.flush();
}

fn base_seq() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
        * 1000
}

pub fn producer(topic: &str, id: u64) {
    let mut ch =
        LeanChannel::open_qos(topic, MSG, qos_for(topic, id), true, false).expect("open producer");
    ch.register_publisher().expect("register publisher");
    let mut seq = base_seq();
    let mut sent = 0u64;
    let mut last = Instant::now();
    loop {
        seq += 1;
        let msg = encode(seq, id);
        let mut standby = false;
        while ch.publish(&msg).is_err() {
            standby = true;
            std::thread::sleep(Duration::from_micros(100));
            if last.elapsed() >= Duration::from_secs(1) {
                sent += 1;
                report(&format!("prod:{topic}:{id}"), sent);
                last = Instant::now();
            }
        }
        if !standby {
            sent += 1;
        }
        if sent & 63 == 0 {
            std::thread::sleep(Duration::from_micros(500));
        }
        if last.elapsed() >= Duration::from_secs(1) {
            report(&format!("prod:{topic}:{id}"), sent);
            last = Instant::now();
        }
    }
}

pub fn consumer(topic: &str, id: u64) {
    let mut ch =
        LeanChannel::open_qos(topic, MSG, qos_for(topic, 0), true, true).expect("open consumer");
    let mut buf = [0u8; MSG];
    let mut high = std::collections::HashMap::<u64, u64>::new();
    let mut got = 0u64;
    let mut last = Instant::now();
    loop {
        match ch.try_read(&mut buf).expect("read") {
            LeanRead::Message => {
                let (seq, producer) = verify(&buf);
                let prev = high.insert(producer, seq).unwrap_or(0);
                assert!(
                    seq > prev,
                    "out of order from {producer}: {seq} after {prev}"
                );
                got += 1;
            }
            LeanRead::Empty => std::thread::sleep(Duration::from_micros(50)),
        }
        if last.elapsed() >= Duration::from_secs(1) {
            report(&format!("cons:{topic}:{id}"), got);
            last = Instant::now();
        }
    }
}

pub fn rpc_server() {
    let mut server = RpcServer::open("soak_rpc", RPC_PAYLOAD, RPC_SLOTS).expect("open server");
    let mut buf = [0u8; RPC_PAYLOAD];
    let mut served = 0u64;
    let mut last = Instant::now();
    loop {
        if let Some(req) = server.poll(&mut buf).expect("poll") {
            let _ = server.reply(req.client_id, req.sequence, &buf[..req.len]);
            served += 1;
        } else {
            server.wait_request(2000, Some(Duration::from_millis(20)));
        }
        if last.elapsed() >= Duration::from_secs(1) {
            report("rpc:server", served);
            last = Instant::now();
        }
    }
}

pub fn rpc_client() {
    let mut client = RpcClient::open("soak_rpc", RPC_PAYLOAD, RPC_SLOTS).expect("open client");
    let mut out = [0u8; RPC_PAYLOAD];
    let mut n = base_seq();
    let mut ok = 0u64;
    let mut last = Instant::now();
    loop {
        n += 1;
        let req = n.to_le_bytes();
        if let Ok(len) = client.call(&req, &mut out, Duration::from_millis(500)) {
            assert_eq!(
                &out[..len],
                &req[..],
                "rpc reply does not match its request"
            );
            ok += 1;
        }
        std::thread::sleep(Duration::from_micros(500));
        if last.elapsed() >= Duration::from_secs(1) {
            report("rpc:client", ok);
            last = Instant::now();
        }
    }
}
