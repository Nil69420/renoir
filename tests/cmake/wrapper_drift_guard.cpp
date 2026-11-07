/**
 * @file wrapper_drift_guard.cpp
 * @brief Compile-and-run guard against renoir.hpp / renoir.h drifting from the
 *        real Rust FFI surface.
 *
 * Every wrapper class in renoir.hpp is instantiated and exercised against the
 * real built library here. If renoir.hpp references a symbol the library
 * doesn't export, this fails to link; if a struct layout diverges, the
 * runtime checks fail.
 */

#include "renoir.hpp"

#include <cassert>
#include <cstdio>
#include <iostream>
#include <string>
#include <vector>
#include <unistd.h>

using namespace renoir;

static void test_version() {
    auto [major, minor, patch] = getVersion();
    std::string version = getVersionString();
    assert(!version.empty());
    assert(major > 0 || minor > 0 || patch > 0);
    std::cout << "  version " << version << " (" << major << "." << minor << "." << patch
              << ")\n";
}

static void test_ring_buffer() {
    RingBuffer ring(16);
    assert(ring.isEmpty());
    assert(ring.availableSpace() > 0);
    assert(!ring.isFull());

    ring.push(42);
    assert(!ring.isEmpty());
    assert(ring.tryPush(43));

    uint8_t data = 0;
    assert(ring.tryPop(&data) && data == 42);
    (void)data;
    assert(ring.peek(&data) && data == 43);
    assert(ring.pop() == 43);
    assert(ring.isEmpty());

    ring.reset();
    RenoirRingStats stats = ring.stats();
    (void)stats;
    assert(stats.capacity == 16);
    std::cout << "  ring buffer ok\n";
}

static void test_manager_and_region() {
    SharedMemoryManager manager;
    assert(manager.handle() != nullptr);

    RegionConfig config;
    config.name = "drift_guard_region";
    config.size = 64 * 1024 * 1024;
    config.use_memfd = true;

    SharedMemoryRegion region = manager.createRegion(config);
    assert(region.size() == 64 * 1024 * 1024);
    assert(!region.name().empty());
    (void)region.stats();

    std::vector<std::string> regions = manager.listRegions();
    assert(!regions.empty());
    assert(manager.regionCount() >= 1);
    assert(manager.totalMemoryUsage() >= 64 * 1024 * 1024);

    SharedMemoryRegion fetched = manager.getRegion("drift_guard_region");
    assert(fetched.handle() != nullptr);

    manager.flushAll();
    manager.removeRegion("drift_guard_region");
    std::cout << "  manager/region ok\n";
}

static void test_buffer_pool() {
    SharedMemoryManager manager;

    RegionConfig config;
    config.name = "drift_guard_pool_region";
    config.size = 64 * 1024 * 1024;
    config.use_memfd = true;
    manager.createRegion(config);

    BufferPoolConfig pool_config;
    pool_config.name = "drift_guard_pool";
    pool_config.buffer_size = 4096;
    pool_config.initial_count = 4;
    pool_config.max_count = 16;

    BufferPool pool = manager.createBufferPool("drift_guard_pool_region", pool_config);
    assert(pool.availableCount() >= 1);
    assert(pool.allocatedCount() == 0);
    (void)pool.canExpand();

    Buffer buffer = pool.getBuffer();
    assert(pool.allocatedCount() >= 1);
    assert(buffer.capacity() >= 4096);
    assert(buffer.size() == buffer.capacity());

    std::vector<uint8_t> payload = {'d', 'r', 'i', 'f', 't'};
    buffer.writeBytes(payload);
    assert(buffer.size() == payload.size());
    std::vector<uint8_t> read_back = buffer.readBytes();
    assert(read_back == payload);
    assert(buffer.sequence() > 0);

    (void)pool.stats();
    std::cout << "  buffer pool ok\n";
}

static void test_topics() {
    TopicManager manager;
    assert(manager.topicCount() == 0);

    TopicOptions options;
    options.pattern = 0; // SPSC
    options.ring_capacity = 16;
    options.max_payload_size = 1024;

    RenoirTopicId topic_id = manager.registerTopic("/drift/guard", options);
    assert(topic_id > 0);
    (void)topic_id;
    assert(manager.lookupTopic("/drift/guard") == topic_id);
    assert(manager.topicCount() == 1);
    assert(!manager.listTopics().empty());

    {
        Publisher publisher = manager.createPublisher("/drift/guard");
        assert(publisher.topicName() == "/drift/guard");
        (void)publisher.hasSubscribers();
        (void)publisher.stats();

        Subscriber subscriber = manager.createSubscriber("/drift/guard");
        assert(subscriber.topicName() == "/drift/guard");
        assert(subscriber.pendingCount() == 0);
        assert(!subscriber.hasMessages());

        RenoirSequenceNumber seq = 0;
        std::vector<uint8_t> payload = {'h', 'e', 'l', 'l', 'o'};
        publisher.publish(payload, &seq);
        assert(seq > 0);
        assert(publisher.tryPublish({'w', 'o', 'r', 'l', 'd'}));
        assert(subscriber.hasMessages());

        ReceivedMessage message = subscriber.readNext(1000);
        assert(message.size() == payload.size());
        assert(message.data()[0] == 'h');
        assert(message.sequence() > 0);
        assert(message.timestampNs() > 0);

        // After real messages have flowed, the stats pipeline must report
        // real (non-zero) values, not the Stage-0 hardcoded zeros.
        RenoirTopicStats pub_stats = publisher.stats();
        RenoirTopicStats sub_stats = subscriber.stats();
        RenoirTopicStats topic_stats = manager.topicStats("/drift/guard");
#ifdef NDEBUG
        (void)pub_stats;
        (void)sub_stats;
        (void)topic_stats;
#else
        assert(pub_stats.messages_published >= 2);
        assert(pub_stats.average_latency_ns > 0);
        assert(pub_stats.peak_latency_ns > 0);
        assert(pub_stats.throughput_mbps > 0.0);

        assert(sub_stats.messages_consumed >= 1);
        assert(sub_stats.average_latency_ns > 0);

        assert(topic_stats.messages_published >= 2);
        assert(topic_stats.messages_consumed >= 1);
        assert(topic_stats.average_latency_ns > 0);
        assert(topic_stats.throughput_mbps > 0.0);
#endif
    }

    // Subscriber 2 (SPMC-style multiple consumers is not allowed for SPSC;
    // use MPMC pattern for fan-out)
    {
        TopicOptions mpmc_options = options;
        mpmc_options.pattern = 3; // MPMC
        manager.registerTopic("/drift/mpmc", mpmc_options);

        Publisher mpmc_pub = manager.createPublisher("/drift/mpmc");
        Subscriber mpmc_sub = manager.createSubscriber("/drift/mpmc");
        mpmc_pub.publish({'m', 'p', 'm', 'c'});
        ReceivedMessage mpmc_msg = mpmc_sub.readNext(1000);
        assert(mpmc_msg.size() == 4);
    }

    manager.removeTopic("/drift/mpmc");
    manager.removeTopic("/drift/guard");
    std::cout << "  topics ok\n";
}

static void test_broadcast_zero_copy() {
    TopicManager manager;
    std::string name = "/drift/zc_" + std::to_string(getpid());

    TopicOptions options;
    options.pattern = 1; // SPMC broadcast
    options.ring_capacity = 1024;
    options.max_payload_size = 4096;
    manager.registerTopic(name, options);

    Publisher publisher = manager.createPublisher(name);
    Subscriber sub_a = manager.createSubscriber(name);
    Subscriber sub_b = manager.createSubscriber(name);

    std::vector<uint8_t> buf(4096);
    size_t len = 0;
    assert(sub_a.readInto(buf.data(), buf.size(), &len) == ReadResult::Empty);

    {
        Reservation r = publisher.reserve(100);
        assert(r.data() != nullptr);
        for (size_t i = 0; i < 100; ++i) r.data()[i] = 0x5A;
        RenoirSequenceNumber seq = r.commit(100);
        assert(seq == 1);
        (void)seq;
    }
    for (Subscriber* sub : {&sub_a, &sub_b}) {
        RenoirMessageMetadata meta = {};
        assert(sub->readInto(buf.data(), buf.size(), &len, &meta) == ReadResult::Message);
        assert(len == 100);
        assert(buf[0] == 0x5A && buf[99] == 0x5A);
        assert(meta.sequence_number == 1);
        (void)meta;
    }

    {
        Reservation dropped = publisher.reserve(50);
        (void)dropped;
    }
    {
        Reservation r = publisher.reserve(10);
        r.data()[0] = 1;
        r.commit(10);
    }
    uint8_t tiny[4];
    assert(sub_a.readInto(tiny, sizeof(tiny), &len) == ReadResult::TooSmall);
    assert(len == 10);
    assert(sub_a.readInto(buf.data(), buf.size(), &len) == ReadResult::Message);
    assert(sub_a.overruns() == 0);
    (void)tiny;
    std::cout << "  broadcast zero-copy ok\n";
}

int main() {
    std::cout << "Renoir C++ wrapper drift guard\n";
    test_version();
    test_ring_buffer();
    test_manager_and_region();
    test_buffer_pool();
    test_topics();
    test_broadcast_zero_copy();
    std::cout << "drift guard passed\n";
    return 0;
}
