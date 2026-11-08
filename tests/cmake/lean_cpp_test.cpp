#include "renoir_lean.hpp"

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

    const std::string path = std::string(dir) + "/renoir_" + name;
    unlink(path.c_str());
    rmdir(dir);
    std::puts("lean_cpp_test OK");
    return 0;
}
