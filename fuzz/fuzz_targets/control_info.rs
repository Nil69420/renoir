#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic_rings::{BroadcastTopicRing, MPMCTopicRing, SPSCTopicRing};

fuzz_target!(|data: &[u8]| {
    let _ = BroadcastTopicRing::read_control_info(data);
    let _ = SPSCTopicRing::read_control_info(data);
    let _ = SPSCTopicRing::read_pending_header(data);
    let _ = MPMCTopicRing::read_control_info(data);
    let _ = MPMCTopicRing::read_pending_header(data);
});
