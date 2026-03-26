/**
 * @file topic_comprehensive_example.cpp
 * @brief Comprehensive example showing topics + control region + ring buffers integration
 * 
 * This example demonstrates:
 * - Topic-based pub/sub messaging
 * - Control region metadata tracking
 * - Ring buffer statistics monitoring
 * - Multi-threaded producer-consumer patterns
 * - Event-driven notifications (Linux)
 * - Proper resource cleanup
 */

#include <iostream>
#include <string>
#include <vector>
#include <thread>
#include <atomic>
#include <chrono>
#include <iomanip>
#include <cstring>

#ifdef __linux__
#include <sys/epoll.h>
#include <unistd.h>
#endif

extern "C" {
#include "renoir.h"
}

class TopicComprehensiveExample {
private:
    RenoirTopicManagerHandle topic_manager = nullptr;
    RenoirPublisherHandle pub_sensor = nullptr;
    RenoirPublisherHandle pub_telemetry = nullptr;
    RenoirSubscriberHandle sub_sensor = nullptr;
    RenoirSubscriberHandle sub_telemetry = nullptr;
    std::atomic<bool> running{true};
    std::atomic<uint64_t> messages_sent{0};
    std::atomic<uint64_t> messages_received{0};

    void print_header(const std::string& title) {
        std::cout << "\n" << std::string(70, '=') << std::endl;
        std::cout << "  " << title << std::endl;
        std::cout << std::string(70, '=') << std::endl;
    }

    void print_section(const std::string& section) {
        std::cout << "\n>>> " << section << std::endl;
    }

    void check_error(RenoirErrorCode error, const std::string& operation) {
        if (error != RenoirErrorCode::Success) {
            std::cerr << "ERROR in " << operation << ": " << static_cast<int>(error) << std::endl;
            cleanup();
            exit(1);
        }
    }

    void setup_topic_infrastructure() {
        print_section("Setting up Topic Infrastructure");

        // Create topic manager
        topic_manager = renoir_topic_manager_create();
        if (topic_manager == nullptr) {
            std::cerr << "Failed to create topic manager" << std::endl;
            exit(1);
        }
        std::cout << "  ✓ Topic manager created" << std::endl;

        // Register sensor data topic (SPSC - single producer, single consumer)
        RenoirTopicOptions sensor_opts = {};
        sensor_opts.pattern = 0;  // SPSC
        sensor_opts.ring_capacity = 32;  // Power of 2
        sensor_opts.max_payload_size = 512;
        sensor_opts.use_shared_pool = false;
        sensor_opts.enable_notifications = true;

        RenoirTopicId sensor_id;
        auto result = renoir_topic_register(topic_manager, "/sensors/imu", &sensor_opts, &sensor_id);
        check_error(result, "register /sensors/imu");
        std::cout << "  ✓ Registered /sensors/imu (ID: " << sensor_id << ", SPSC ring)" << std::endl;

        // Register telemetry topic (MPMC - multiple producers, multiple consumers)
        RenoirTopicOptions telemetry_opts = {};
        telemetry_opts.pattern = 3;  // MPMC
        telemetry_opts.ring_capacity = 64;
        telemetry_opts.max_payload_size = 256;
        telemetry_opts.use_shared_pool = false;
        telemetry_opts.enable_notifications = true;

        RenoirTopicId telemetry_id;
        result = renoir_topic_register(topic_manager, "/telemetry/status", &telemetry_opts, &telemetry_id);
        check_error(result, "register /telemetry/status");
        std::cout << "  ✓ Registered /telemetry/status (ID: " << telemetry_id << ", MPMC ring)" << std::endl;

        size_t topic_count = renoir_topic_count(topic_manager);
        std::cout << "  ✓ Total topics registered: " << topic_count << std::endl;
    }

    void create_publishers_and_subscribers() {
        print_section("Creating Publishers and Subscribers");

        RenoirPublisherOptions pub_opts = {};
        pub_opts.batch_size = 1;
        pub_opts.timeout_ms = 100;
        pub_opts.priority = 0;
        pub_opts.enable_batching = false;

        // Sensor publisher
        auto result = renoir_publisher_create(topic_manager, "/sensors/imu", &pub_opts, &pub_sensor);
        check_error(result, "create sensor publisher");
        std::cout << "  ✓ Sensor publisher created" << std::endl;

        // Telemetry publisher
        result = renoir_publisher_create(topic_manager, "/telemetry/status", &pub_opts, &pub_telemetry);
        check_error(result, "create telemetry publisher");
        std::cout << "  ✓ Telemetry publisher created" << std::endl;

        RenoirSubscriberOptions sub_opts = {};
        sub_opts.mode = 0;
        sub_opts.batch_size = 1;
        sub_opts.timeout_ms = 1000;
        sub_opts.queue_depth = 32;
        sub_opts.enable_filtering = false;

        // Sensor subscriber
        result = renoir_subscriber_create(topic_manager, "/sensors/imu", &sub_opts, &sub_sensor);
        check_error(result, "create sensor subscriber");
        std::cout << "  ✓ Sensor subscriber created" << std::endl;

        // Telemetry subscriber
        result = renoir_subscriber_create(topic_manager, "/telemetry/status", &sub_opts, &sub_telemetry);
        check_error(result, "create telemetry subscriber");
        std::cout << "  ✓ Telemetry subscriber created" << std::endl;
    }

    void publish_sensor_data() {
        print_section("Publishing Sensor Data (Producer Thread)");

        for (int i = 0; i < 10 && running; i++) {
            // Simulate IMU sensor data
            struct ImuData {
                double accel[3];
                double gyro[3];
                uint64_t timestamp_ns;
            } imu_data;

            imu_data.accel[0] = 9.81 + (i * 0.01);
            imu_data.accel[1] = 0.05 * i;
            imu_data.accel[2] = 0.02 * i;
            imu_data.gyro[0] = 0.001 * i;
            imu_data.gyro[1] = 0.002 * i;
            imu_data.gyro[2] = 0.003 * i;
            imu_data.timestamp_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                std::chrono::steady_clock::now().time_since_epoch()
            ).count();

            RenoirMessageMetadata metadata = {};
            metadata.timestamp_ns = imu_data.timestamp_ns;
            metadata.sequence_number = i;
            metadata.source_id = 1;
            metadata.message_type = 100;  // IMU message type
            metadata.flags = 0;

            RenoirSequenceNumber seq_out = 0;
            auto result = renoir_publish(
                pub_sensor,
                reinterpret_cast<const uint8_t*>(&imu_data),
                sizeof(imu_data),
                &metadata,
                &seq_out
            );

            if (result == RenoirErrorCode::Success) {
                messages_sent++;
                std::cout << "  [Producer] Published IMU #" << i 
                          << " (seq=" << seq_out << ")" << std::endl;
            } else {
                std::cerr << "  [Producer] Publish failed: " << static_cast<int>(result) << std::endl;
            }

            std::this_thread::sleep_for(std::chrono::milliseconds(50));
        }
    }

    void consume_sensor_data() {
        print_section("Consuming Sensor Data (Consumer Thread)");

#ifdef __linux__
        // Get event fd for efficient waiting
        int event_fd = -1;
        auto result = renoir_subscriber_get_event_fd(sub_sensor, &event_fd);
        if (result == RenoirErrorCode::Success && event_fd >= 0) {
            std::cout << "  [Consumer] Using event-driven mode (fd=" << event_fd << ")" << std::endl;
        }
#endif

        while (running) {
            RenoirReceivedMessage msg = {};
            auto result = renoir_subscribe_read_next(sub_sensor, &msg, 100);  // 100ms timeout

            if (result == RenoirErrorCode::Success) {
                messages_received++;
                auto* imu_data = reinterpret_cast<const struct ImuData*>(msg.payload_ptr);
                
                std::cout << "  [Consumer] Received IMU (seq=" << msg.metadata.sequence_number << "): "
                          << "accel=[" << std::fixed << std::setprecision(3)
                          << imu_data->accel[0] << ", "
                          << imu_data->accel[1] << ", "
                          << imu_data->accel[2] << "]" << std::endl;

                renoir_message_release(msg.handle);
            } else if (result == RenoirErrorCode::BufferEmpty) {
                // Timeout - check if we should continue
                if (messages_received >= messages_sent && messages_sent > 0) {
                    break;
                }
            } else {
                std::cerr << "  [Consumer] Read failed: " << static_cast<int>(result) << std::endl;
                break;
            }
        }
    }

    struct ImuData {
        double accel[3];
        double gyro[3];
        uint64_t timestamp_ns;
    };

    void display_topic_statistics() {
        print_section("Topic and Ring Buffer Statistics");

        // Sensor topic stats
        RenoirTopicStats sensor_stats = {};
        auto result = renoir_topic_stats(topic_manager, "/sensors/imu", &sensor_stats);
        if (result == RenoirErrorCode::Success) {
            std::cout << "\n  /sensors/imu Statistics:" << std::endl;
            std::cout << "    Messages Published: " << sensor_stats.messages_published << std::endl;
            std::cout << "    Messages Consumed:  " << sensor_stats.messages_consumed << std::endl;
            std::cout << "    Messages Dropped:   " << sensor_stats.messages_dropped << std::endl;
            std::cout << "    Publisher Count:    " << sensor_stats.publisher_count << std::endl;
            std::cout << "    Subscriber Count:   " << sensor_stats.subscriber_count << std::endl;
        }

        // Telemetry topic stats
        RenoirTopicStats telemetry_stats = {};
        result = renoir_topic_stats(topic_manager, "/telemetry/status", &telemetry_stats);
        if (result == RenoirErrorCode::Success) {
            std::cout << "\n  /telemetry/status Statistics:" << std::endl;
            std::cout << "    Messages Published: " << telemetry_stats.messages_published << std::endl;
            std::cout << "    Messages Consumed:  " << telemetry_stats.messages_consumed << std::endl;
            std::cout << "    Messages Dropped:   " << telemetry_stats.messages_dropped << std::endl;
            std::cout << "    Publisher Count:    " << telemetry_stats.publisher_count << std::endl;
            std::cout << "    Subscriber Count:   " << telemetry_stats.subscriber_count << std::endl;
        }

        // Overall counts
        std::cout << "\n  Overall Counters:" << std::endl;
        std::cout << "    Total Sent:     " << messages_sent.load() << std::endl;
        std::cout << "    Total Received: " << messages_received.load() << std::endl;
        std::cout << "    Success Rate:   " << std::fixed << std::setprecision(1)
                  << (messages_sent > 0 ? (messages_received * 100.0 / messages_sent) : 0) << "%" << std::endl;
    }

    void cleanup() {
        print_section("Cleanup");

        if (sub_sensor) {
            renoir_subscriber_destroy(sub_sensor);
            std::cout << "  ✓ Sensor subscriber destroyed" << std::endl;
        }

        if (sub_telemetry) {
            renoir_subscriber_destroy(sub_telemetry);
            std::cout << "  ✓ Telemetry subscriber destroyed" << std::endl;
        }

        if (pub_sensor) {
            renoir_publisher_destroy(pub_sensor);
            std::cout << "  ✓ Sensor publisher destroyed" << std::endl;
        }

        if (pub_telemetry) {
            renoir_publisher_destroy(pub_telemetry);
            std::cout << "  ✓ Telemetry publisher destroyed" << std::endl;
        }

        if (topic_manager) {
            renoir_topic_manager_destroy(topic_manager);
            std::cout << "  ✓ Topic manager destroyed" << std::endl;
        }
    }

public:
    void run() {
        print_header("Renoir Topic System - Comprehensive Example");

        std::cout << "\nThis example demonstrates:\n"
                  << "  - Topic registration with different ring types (SPSC, MPMC)\n"
                  << "  - Publisher/Subscriber creation and lifecycle\n"
                  << "  - Multi-threaded message passing\n"
                  << "  - Topic statistics and monitoring\n"
                  << "  - Event-driven notifications (Linux)\n"
                  << "  - Proper resource cleanup\n";

        setup_topic_infrastructure();
        create_publishers_and_subscribers();

        // Start producer and consumer threads
        print_section("Running Multi-threaded Pub/Sub Test");
        std::cout << "  Starting producer and consumer threads..." << std::endl;

        std::thread producer_thread(&TopicComprehensiveExample::publish_sensor_data, this);
        std::thread consumer_thread(&TopicComprehensiveExample::consume_sensor_data, this);

        producer_thread.join();
        consumer_thread.join();

        display_topic_statistics();
        cleanup();

        print_header("Example Completed Successfully!");
    }
};

int main() {
    try {
        TopicComprehensiveExample example;
        example.run();
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "Exception: " << e.what() << std::endl;
        return 1;
    }
}
