#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <renoir.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

#define MAX_CAMS 8

typedef struct {
    int cams;
    int fps;
    int imu_hz;
    uint32_t width;
    uint32_t height;
    int duration_s;
    int slow_ms;
    int latest;
    int start_delay_s;
    const char *label;
} config_t;

typedef struct {
    int stream;
    config_t cfg;
    RenoirTopicManagerHandle mgr;
    size_t payload;
    uint64_t start_ns;
    uint64_t published;
    uint64_t errors;
    uint32_t *work_ns;
    uint32_t *late_ns;
    size_t samples;
    uint64_t received;
    uint64_t lost;
    uint64_t disorder;
    uint64_t torn;
    uint64_t invalid;
    uint64_t overruns;
    int failed;
} stream_t;

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static void sleep_until(uint64_t target_ns) {
    struct timespec ts;
    ts.tv_sec = (time_t)(target_ns / 1000000000ull);
    ts.tv_nsec = (long)(target_ns % 1000000000ull);
    while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts, NULL) == EINTR) {
    }
}

static int cmp_u32(const void *a, const void *b) {
    uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
    return (x > y) - (x < y);
}

static void report_percentiles(const char *name, uint32_t *v, size_t n) {
    if (n == 0) {
        printf("  %-14s no samples\n", name);
        return;
    }
    qsort(v, n, sizeof(uint32_t), cmp_u32);
    printf("  %-14s p50 %8.1f us  p99 %8.1f us  p99.9 %8.1f us  max %8.1f us  (n=%zu)\n", name,
           v[n / 2] / 1e3, v[n * 99 / 100] / 1e3, v[n * 999 / 1000] / 1e3, v[n - 1] / 1e3, n);
}

static size_t ring_kib_for(size_t payload, size_t want_slots) {
    size_t stride = ((payload + 128 + 63) / 64) * 64;
    size_t kib = (stride * want_slots + 1023) / 1024;
    size_t p = 1;
    while (p < kib) {
        p <<= 1;
    }
    return p;
}

static void topic_name(char *out, size_t n, int stream) {
    if (stream < MAX_CAMS) {
        snprintf(out, n, "/soak/cam%d/image", stream);
    } else {
        snprintf(out, n, "/soak/imu");
    }
}

static int register_topic(RenoirTopicManagerHandle mgr, const char *name, size_t payload,
                          size_t want_slots) {
    RenoirTopicOptions o;
    memset(&o, 0, sizeof(o));
    o.pattern = 1;
    o.ring_capacity = ring_kib_for(payload, want_slots);
    o.max_payload_size = payload;
    RenoirTopicId id;
    return renoir_topic_register(mgr, name, &o, &id) == RenoirErrorCode_Success ? 0 : -1;
}

static void *camera_producer(void *arg) {
    stream_t *s = arg;
    char name[64];
    topic_name(name, sizeof(name), s->stream);
    RenoirPublisherHandle pub = NULL;
    if (renoir_publisher_create(s->mgr, name, NULL, &pub) != RenoirErrorCode_Success) {
        s->failed = 1;
        return NULL;
    }
    uint64_t period = 1000000000ull / (uint64_t)s->cfg.fps;
    uint64_t frames = (uint64_t)s->cfg.duration_s * (uint64_t)s->cfg.fps;
    s->work_ns = calloc(frames, sizeof(uint32_t));
    s->late_ns = calloc(frames, sizeof(uint32_t));
    if (!s->work_ns || !s->late_ns) {
        s->failed = 1;
        return NULL;
    }
    size_t pixels = (size_t)s->cfg.width * s->cfg.height;
    for (uint64_t i = 0; i < frames; i++) {
        uint64_t target = s->start_ns + i * period;
        sleep_until(target);
        uint64_t t0 = now_ns();
        uint8_t *slot = NULL;
        RenoirReservedBufferHandle h = NULL;
        if (renoir_publish_reserve(pub, s->payload, &slot, &h) != RenoirErrorCode_Success) {
            s->errors++;
            continue;
        }
        RenoirImageFrameHeader *hdr = (RenoirImageFrameHeader *)slot;
        renoir_image_frame_header_init(hdr, s->cfg.width, s->cfg.height, 0,
                                       RENOIR_ENCODING_MONO8, t0, (uint32_t)s->stream, i + 1);
        hdr->fx = 458.654;
        hdr->fy = 457.296;
        hdr->cx = 367.215;
        hdr->cy = 248.375;
        memset(slot + hdr->pixel_offset, (int)((i + 1) & 0xFF), pixels);
        if (renoir_publish_commit(pub, h, s->payload, NULL, NULL) != RenoirErrorCode_Success) {
            s->errors++;
            continue;
        }
        uint64_t t1 = now_ns();
        s->published++;
        s->work_ns[s->samples] = (uint32_t)(t1 - t0);
        s->late_ns[s->samples] = t0 > target ? (uint32_t)(t0 - target) : 0;
        s->samples++;
    }
    renoir_publisher_destroy(pub);
    return NULL;
}

static void *imu_producer(void *arg) {
    stream_t *s = arg;
    RenoirPublisherHandle pub = NULL;
    if (renoir_publisher_create(s->mgr, "/soak/imu", NULL, &pub) != RenoirErrorCode_Success) {
        s->failed = 1;
        return NULL;
    }
    uint64_t period = 1000000000ull / (uint64_t)s->cfg.imu_hz;
    uint64_t count = (uint64_t)s->cfg.duration_s * (uint64_t)s->cfg.imu_hz;
    s->work_ns = calloc(count, sizeof(uint32_t));
    s->late_ns = calloc(count, sizeof(uint32_t));
    if (!s->work_ns || !s->late_ns) {
        s->failed = 1;
        return NULL;
    }
    for (uint64_t i = 0; i < count; i++) {
        uint64_t target = s->start_ns + i * period;
        sleep_until(target);
        uint64_t t0 = now_ns();
        uint8_t *slot = NULL;
        RenoirReservedBufferHandle h = NULL;
        if (renoir_publish_reserve(pub, sizeof(RenoirImuSample), &slot, &h) !=
            RenoirErrorCode_Success) {
            s->errors++;
            continue;
        }
        double gyro[3] = {(double)(i + 1), 0.0, 0.0};
        double accel[3] = {0.0, 0.0, 9.81};
        renoir_imu_sample_init((RenoirImuSample *)slot, t0, gyro, accel, 0, i + 1);
        if (renoir_publish_commit(pub, h, sizeof(RenoirImuSample), NULL, NULL) !=
            RenoirErrorCode_Success) {
            s->errors++;
            continue;
        }
        uint64_t t1 = now_ns();
        s->published++;
        s->work_ns[s->samples] = (uint32_t)(t1 - t0);
        s->late_ns[s->samples] = t0 > target ? (uint32_t)(t0 - target) : 0;
        s->samples++;
    }
    renoir_publisher_destroy(pub);
    return NULL;
}

static void *consumer(void *arg) {
    stream_t *s = arg;
    char name[64];
    topic_name(name, sizeof(name), s->stream);
    int is_imu = s->stream >= MAX_CAMS;
    RenoirSubscriberHandle sub = NULL;
    if (renoir_subscriber_create(s->mgr, name, NULL, &sub) != RenoirErrorCode_Success) {
        s->failed = 1;
        return NULL;
    }
    uint64_t rate = is_imu ? (uint64_t)s->cfg.imu_hz : (uint64_t)s->cfg.fps;
    uint64_t cap = (uint64_t)(s->cfg.duration_s + 4) * rate;
    s->work_ns = calloc(cap, sizeof(uint32_t));
    uint8_t *buf = malloc(s->payload);
    if (!s->work_ns || !buf) {
        s->failed = 1;
        return NULL;
    }
    size_t pixels = (size_t)s->cfg.width * s->cfg.height;
    uint64_t deadline = s->start_ns + (uint64_t)(s->cfg.duration_s + 3) * 1000000000ull;
    uint64_t last = 0;
    while (now_ns() < deadline) {
        if (renoir_subscribe_wait(sub, 100) != RenoirErrorCode_Success) {
            continue;
        }
        for (;;) {
            if (!is_imu && s->cfg.latest) {
                renoir_subscriber_skip_to_latest(sub, 1, NULL);
            }
            size_t len = 0;
            RenoirMessageMetadata meta;
            memset(&meta, 0, sizeof(meta));
            RenoirErrorCode rc = renoir_subscribe_read_into(sub, buf, s->payload, &len, &meta);
            if (rc != RenoirErrorCode_Success) {
                break;
            }
            uint64_t t = now_ns();
            uint64_t seq, stamp;
            if (is_imu) {
                const RenoirImuSample *m = (const RenoirImuSample *)buf;
                if (renoir_imu_sample_validate(buf, len) != RenoirErrorCode_Success) {
                    s->invalid++;
                    continue;
                }
                seq = m->seq;
                stamp = m->stamp_ns;
                if (m->gyro[0] != (double)seq) {
                    s->torn++;
                }
            } else {
                const RenoirImageFrameHeader *h = (const RenoirImageFrameHeader *)buf;
                if (renoir_image_frame_validate(buf, len) != RenoirErrorCode_Success) {
                    s->invalid++;
                    continue;
                }
                seq = h->frame_seq;
                stamp = h->stamp_ns;
                const uint8_t *px = buf + h->pixel_offset;
                uint8_t want = (uint8_t)(seq & 0xFF);
                int bad = px[0] != want || px[pixels - 1] != want;
                for (size_t k = 1; k < 64 && !bad; k++) {
                    bad = px[(pixels / 64) * k] != want;
                }
                s->torn += (uint64_t)bad;
            }
            if (last != 0 && seq <= last) {
                s->disorder++;
            } else if (last != 0 && seq != last + 1) {
                s->lost += seq - last - 1;
            }
            last = seq;
            if (s->samples < cap) {
                s->work_ns[s->samples++] = (uint32_t)(t > stamp ? t - stamp : 0);
            }
            s->received++;
            if (!is_imu && s->cfg.slow_ms > 0) {
                struct timespec d = {0, (long)s->cfg.slow_ms * 1000000L};
                nanosleep(&d, NULL);
            }
        }
    }
    renoir_subscriber_overruns(sub, &s->overruns);
    renoir_subscriber_destroy(sub);
    free(buf);
    return NULL;
}

static int parse_args(int argc, char **argv, config_t *c, const char **role) {
    if (argc < 2) {
        return -1;
    }
    *role = argv[1];
    c->cams = 2;
    c->fps = 120;
    c->imu_hz = 1000;
    c->width = 1280;
    c->height = 720;
    c->duration_s = 30;
    c->slow_ms = 0;
    c->latest = 0;
    c->start_delay_s = 3;
    c->label = *role;
    for (int i = 2; i + 1 < argc; i += 2) {
        const char *k = argv[i];
        const char *v = argv[i + 1];
        if (!strcmp(k, "--cams")) c->cams = atoi(v);
        else if (!strcmp(k, "--fps")) c->fps = atoi(v);
        else if (!strcmp(k, "--imu-hz")) c->imu_hz = atoi(v);
        else if (!strcmp(k, "--width")) c->width = (uint32_t)atoi(v);
        else if (!strcmp(k, "--height")) c->height = (uint32_t)atoi(v);
        else if (!strcmp(k, "--duration")) c->duration_s = atoi(v);
        else if (!strcmp(k, "--slow-ms")) c->slow_ms = atoi(v);
        else if (!strcmp(k, "--latest")) c->latest = atoi(v);
        else if (!strcmp(k, "--start-delay")) c->start_delay_s = atoi(v);
        else if (!strcmp(k, "--label")) c->label = v;
        else return -1;
    }
    return (c->cams < 1 || c->cams > MAX_CAMS || c->fps < 1 || c->imu_hz < 1 ||
            c->duration_s < 1 || c->width < 8 || c->height < 8)
               ? -1
               : 0;
}

int main(int argc, char **argv) {
    config_t cfg;
    const char *role = NULL;
    if (parse_args(argc, argv, &cfg, &role) != 0 ||
        (strcmp(role, "producer") && strcmp(role, "consumer"))) {
        fprintf(stderr,
                "usage: %s producer|consumer [--cams N] [--fps F] [--imu-hz H] [--width W]\n"
                "       [--height H] [--duration S] [--slow-ms M] [--latest 0|1] [--start-delay S] [--label L]\n",
                argv[0]);
        return 2;
    }
    int producing = !strcmp(role, "producer");
    size_t frame_bytes = renoir_image_frame_payload_size(cfg.width, cfg.height, 0, RENOIR_ENCODING_MONO8);
    RenoirTopicManagerHandle mgr = renoir_topic_manager_create();
    if (!mgr || frame_bytes == 0) {
        fprintf(stderr, "setup failed\n");
        return 2;
    }
    for (int c = 0; c < cfg.cams; c++) {
        char name[64];
        topic_name(name, sizeof(name), c);
        if (register_topic(mgr, name, frame_bytes, 24) != 0) {
            fprintf(stderr, "register %s failed\n", name);
            return 2;
        }
    }
    if (register_topic(mgr, "/soak/imu", sizeof(RenoirImuSample), 20000) != 0) {
        fprintf(stderr, "register imu failed\n");
        return 2;
    }

    int n = cfg.cams + 1;
    stream_t streams[MAX_CAMS + 1];
    pthread_t th[MAX_CAMS + 1];
    memset(streams, 0, sizeof(streams));
    uint64_t start = now_ns() + (uint64_t)cfg.start_delay_s * 1000000000ull;
    for (int i = 0; i < n; i++) {
        stream_t *s = &streams[i];
        s->stream = (i < cfg.cams) ? i : MAX_CAMS;
        s->cfg = cfg;
        s->mgr = mgr;
        s->payload = (s->stream < MAX_CAMS) ? frame_bytes : sizeof(RenoirImuSample);
        s->start_ns = start;
        void *(*fn)(void *) = producing ? (s->stream < MAX_CAMS ? camera_producer : imu_producer) : consumer;
        pthread_create(&th[i], NULL, fn, s);
    }
    for (int i = 0; i < n; i++) {
        pthread_join(th[i], NULL);
    }

    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    double cpu = (double)(ru.ru_utime.tv_sec + ru.ru_stime.tv_sec) +
                 (double)(ru.ru_utime.tv_usec + ru.ru_stime.tv_usec) / 1e6;
    double wall = (double)cfg.duration_s;
    printf("== %s (%s) cams=%d fps=%d imu=%dHz %ux%u slow=%dms latest=%d\n", cfg.label, role,
           cfg.cams, cfg.fps, cfg.imu_hz, cfg.width, cfg.height, cfg.slow_ms, cfg.latest);
    int verdict = 0;
    for (int i = 0; i < n; i++) {
        stream_t *s = &streams[i];
        int imu = s->stream >= MAX_CAMS;
        if (producing) {
            printf(" %s: published %llu errors %llu\n", imu ? "imu" : "cam", (unsigned long long)s->published,
                   (unsigned long long)s->errors);
            report_percentiles("publish work", s->work_ns, s->samples);
            report_percentiles("schedule late", s->late_ns, s->samples);
            verdict |= (s->errors > 0 || s->failed != 0);
        } else {
            printf(" %s%d: received %llu lost %llu overruns %llu disorder %llu torn %llu invalid %llu\n",
                   imu ? "imu" : "cam", imu ? 0 : s->stream, (unsigned long long)s->received,
                   (unsigned long long)s->lost, (unsigned long long)s->overruns,
                   (unsigned long long)s->disorder, (unsigned long long)s->torn,
                   (unsigned long long)s->invalid);
            report_percentiles("e2e latency", s->work_ns, s->samples);
            int lossy_ok = (cfg.slow_ms > 0 || cfg.latest) && !imu;
            verdict |= (s->torn || s->invalid || s->disorder || s->failed || s->received == 0 ||
                        (!lossy_ok && s->lost));
        }
    }
    printf("  cpu %.2f s over %.0f s run (%.1f%% of one core)  max rss %ld KiB\n", cpu, wall,
           100.0 * cpu / wall, ru.ru_maxrss);
    printf("  RESULT: %s\n", verdict ? "FAIL" : "PASS");
    renoir_topic_manager_destroy(mgr);
    return verdict ? 1 : 0;
}
