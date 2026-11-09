#include "renoir_discovery.hpp"
#include "renoir_frame.hpp"
#include "renoir_lean.hpp"
#include "renoir_rpc.hpp"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <unistd.h>
#include <utility>

struct Imu {
    uint64_t stamp_ns;
    float gyro[3];
    float accel[3];
    uint32_t seq;
};

#define CHECK(cond)                                                  \
    do {                                                             \
        if (!(cond)) {                                               \
            std::fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #cond); \
            return 1;                                                \
        }                                                            \
    } while (0)

int main() {
    char dir[] = "/tmp/renoir_lean_cpp_XXXXXX";
    CHECK(mkdtemp(dir) != nullptr);
    CHECK(setenv("RENOIR_SHM_DIR", dir, 1) == 0);
    const std::string name = "lean_cpp_" + std::to_string(getpid());

    renoir::LeanChannel pub(name, sizeof(Imu), 128);
    renoir::LeanChannel sub(name, sizeof(Imu), 128, true);
    CHECK(sub.messageSize() == sizeof(Imu));


    {
        const std::string stampedName = "lean_cpp_stamped_" + std::to_string(getpid());
        renoir::LeanChannel sp(stampedName, sizeof(Imu), 64, renoir::LeanFlags::StampNanoseconds);
        renoir::LeanChannel ss(stampedName, sizeof(Imu), 64,
                               renoir::LeanFlags::StampNanoseconds | renoir::LeanFlags::FromOldest);
        CHECK(sp.clockKind() == RENOIR_LEAN_CLOCK_NS);
        Imu in{};
        in.seq = 77;
        Imu got{};
        uint64_t stamp = 0;
        uint64_t missed = 0;
        CHECK(!ss.tryReadStamped(got, stamp));
        CHECK(sp.publishAt(555, in) == RenoirErrorCode::RenoirErrorCode_Success);
        CHECK(ss.tryReadStamped(got, stamp) && stamp == 555 && got.seq == 77);
        const uint64_t before = sp.stampNow();
        CHECK(sp.publishNow(in) == RenoirErrorCode::RenoirErrorCode_Success);
        CHECK(ss.tryReadStamped(got, stamp) && stamp >= before && stamp <= sp.stampNow());
        for (int i = 0; i < 200; ++i) {
            CHECK(sp.publishAt(static_cast<uint64_t>(i + 1), in) ==
                  RenoirErrorCode::RenoirErrorCode_Success);
        }
        uint64_t reads = 0;
        uint64_t totalMissed = 0;
        while (ss.tryReadStampedLagged(&got, sizeof got, stamp, missed)) {
            ++reads;
            totalMissed += missed;
        }
        CHECK(totalMissed > 0 && reads + totalMissed == 200);

        const renoir::WaitStrategy all[] = {
            renoir::WaitStrategy::BusySpin, renoir::WaitStrategy::Yielding,
            renoir::WaitStrategy::Sleeping, renoir::WaitStrategy::Blocking,
            renoir::WaitStrategy::PhasedBackoff};
        for (const renoir::WaitStrategy strategy : all) {
            CHECK(!ss.waitWith(strategy, 2000000ULL));
        }
        CHECK(!ss.waitPhased(100000ULL, 500000ULL, 3000000ULL));
    }
    Imu out{};
    CHECK(!sub.tryRead(out));
    for (uint32_t i = 0; i < 50; ++i) {
        Imu in{i * 1000ULL, {1.0F, 2.0F, 3.0F}, {0.0F, 0.0F, 9.8F}, i};
        CHECK(pub.publish(in) == RenoirErrorCode::RenoirErrorCode_Success);
    }
    CHECK(sub.pending() == 50);
    for (uint32_t i = 0; i < 50; ++i) {
        CHECK(sub.tryRead(out));
        CHECK(out.seq == i && out.stamp_ns == i * 1000ULL && out.accel[2] == 9.8F);
    }
    CHECK(!sub.tryRead(out));

    CHECK(!sub.wait(10, 1000000));
    Imu woken{7, {0.0F, 0.0F, 0.0F}, {0.0F, 0.0F, 0.0F}, 7};
    CHECK(pub.publishNotify(&woken, sizeof woken) == RenoirErrorCode::RenoirErrorCode_Success);
    CHECK(sub.wait(0, 1000000));
    CHECK(sub.tryRead(out) && out.seq == 7);

    {
        const std::string batchName = name + "_batch";
        renoir::LeanChannel bp(batchName, sizeof(Imu), 64);
        renoir::LeanChannel bs(batchName, sizeof(Imu), 64, true);
        Imu in[5] = {};
        for (uint32_t i = 0; i < 5; ++i) {
            in[i].seq = i;
        }
        size_t published = 0;
        CHECK(bp.publishBatch(in, sizeof in, published) ==
              RenoirErrorCode::RenoirErrorCode_Success);
        CHECK(published == 5);
        Imu got[8] = {};
        size_t count = 0;
        CHECK(bs.readBatch(got, sizeof got, count) == RenoirErrorCode::RenoirErrorCode_Success);
        CHECK(count == 5 && got[4].seq == 4);
    }

    {
        const std::string rpcName = "rpc_" + name;
        renoir::RpcServer server(rpcName, 32, 16);
        renoir::RpcClient client(rpcName, 32, 16);
        const char ping[] = "ping";
        char reply[32] = {};
        size_t replyLen = 0;
        CHECK(client.call(ping, sizeof ping, reply, sizeof reply, replyLen, 1000000ULL) ==
              RenoirErrorCode::RenoirErrorCode_BufferEmpty);
        renoir::RpcRequestInfo info;
        char buffer[32] = {};
        CHECK(server.poll(buffer, sizeof buffer, info));
        CHECK(info.length == sizeof ping && std::string(buffer) == "ping");
        CHECK(server.reply(info, "pong", 5) == RenoirErrorCode::RenoirErrorCode_Success);
        CHECK(!server.poll(buffer, sizeof buffer, info));
    }

    {
        renoir::TopicManager manager;
        const std::string topic = "/cpp/view_" + std::to_string(getpid());
        renoir::TopicOptions options = {};
        options.pattern = 1;
        options.ring_capacity = 2;
        options.max_payload_size = 256;
        manager.registerTopic(topic, options);
        renoir::Publisher publisher = manager.createPublisher(topic);
        renoir::Subscriber subscriber = manager.createSubscriber(topic);
        RenoirFrameView frame = {};
        CHECK(!subscriber.view(&frame));
        {
            renoir::Reservation r = publisher.reserve(100);
            for (size_t i = 0; i < 100; ++i) r.data()[i] = 0x33;
            r.commit(100);
        }
        CHECK(subscriber.view(&frame));
        CHECK(frame.len == 100 && frame.data[0] == 0x33 && frame.data[99] == 0x33);
        CHECK(subscriber.releaseView(frame));
        CHECK(!subscriber.view(&frame));
    }

    renoir::LeanChannel moved(std::move(sub));
    CHECK(!moved.tryRead(out));
    CHECK(pub.publish(&out, 1) != RenoirErrorCode::RenoirErrorCode_Success);

    bool threw = false;
    try {
        renoir::LeanChannel bad(name, 0, 8);
    } catch (const renoir::Exception&) {
        threw = true;
    }
    CHECK(threw);

    bool listed = false;
    for (const renoir::RingInfo& ring : renoir::discover()) {
        if (ring.name == name) {
            listed = true;
            CHECK(ring.kind == renoir::RingKind::Lean);
            CHECK(ring.hasMode);
            CHECK(ring.msgSize == sizeof(Imu));
            CHECK(ring.slotCount == 128);
            CHECK(ring.livePublishers >= 1);
        }
    }
    CHECK(listed);

    {
        const std::string fname = "frame_cpp_" + std::to_string(getpid());
        renoir::FrameChannel fpub(fname, 2048, 4, renoir::FrameFlags::None);
        renoir::FrameChannel fsub(fname, 2048, 4, renoir::FrameFlags::Consumer);
        CHECK(fsub.consumerId() >= 0);
        CHECK(!fsub.tryView().has_value());
        for (uint8_t i = 0; i < 4; ++i) {
            auto loan = fpub.tryLoan(100 + i);
            CHECK(loan.has_value());
            CHECK(loan->size() == 100U + i);
            for (size_t k = 0; k < loan->size(); ++k) {
                loan->data()[k] = i;
            }
            loan->commit();
        }
        CHECK(!fpub.tryLoan(10).has_value());
        {
            auto view = fsub.tryView();
            CHECK(view.has_value());
            CHECK(view->size() == 100U);
            CHECK(view->data()[0] == 0 && view->intact());
            CHECK(!fpub.tryLoan(10).has_value());
        }
        {
            auto loan = fpub.tryLoan(10);
            CHECK(loan.has_value());
        }
        for (uint8_t i = 1; i < 4; ++i) {
            auto view = fsub.tryView();
            CHECK(view.has_value() && view->size() == 100U + i && view->data()[0] == i);
        }
        CHECK(!fsub.tryView().has_value());
        unlink((std::string(dir) + "/renoir_" + fname).c_str());
    }

    const std::string path = std::string(dir) + "/renoir_" + name;
    unlink(path.c_str());
    rmdir(dir);
    std::puts("lean_cpp_test OK");
    return 0;
}
