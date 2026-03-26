/**
 * @file topic_async_example.cpp
 * @brief Event-driven async topic subscriber example
 * 
 * This example demonstrates:
 * - Event-driven message reception using epoll (Linux)
 * - Non-blocking subscriber operations
 * - Integration with event loops
 * - Multi-threaded producer/consumer with notifications
 */

#include <iostream>
#include <string>
#include <thread>
#include <atomic>
#include <chrono>
#include <cstring>

#ifdef __linux__
#include <sys/epoll.h>
#include <unistd.h>
#endif

extern "C" {
#include "renoir.h"
}

std::atomic<bool> running{true};

void producer_thread(RenoirPublisherHandle publisher) {
    std::cout << "[Producer] Started\n";
    
    for (int i = 0; i < 20 && running; i++) {
        char msg[128];
        snprintf(msg, sizeof(msg), "Async message #%d", i);
        
        RenoirMessageMetadata metadata = {};
        metadata.timestamp_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now().time_since_epoch()
        ).count();
        metadata.sequence_number = i;
        metadata.source_id = 1;
        
        RenoirSequenceNumber seq_out = 0;
        auto result = renoir_publish(publisher, reinterpret_cast<const uint8_t*>(msg),
                                    strlen(msg) + 1, &metadata, &seq_out);
        
        if (result == RenoirErrorCode::Success) {
            std::cout << "[Producer] Sent message #" << i << "\n";
        }
        
        std::this_thread::sleep_for(std::chrono::milliseconds(200));
    }
    std::cout << "[Producer] Finished\n";
}

#ifdef __linux__
void consumer_event_loop(RenoirSubscriberHandle subscriber) {
    std::cout << "[Consumer] Starting event loop\n";
    
    // Get event fd
    int event_fd = -1;
    auto result = renoir_subscriber_get_event_fd(subscriber, &event_fd);
    if (result != RenoirErrorCode::Success || event_fd < 0) {
        std::cerr << "[Consumer] Failed to get event fd\n";
        return;
    }
    std::cout << "[Consumer] Using event fd: " << event_fd << "\n";
    
    // Setup epoll
    int epoll_fd = epoll_create1(0);
    if (epoll_fd < 0) {
        std::cerr << "[Consumer] Failed to create epoll\n";
        return;
    }
    
    struct epoll_event ev;
    ev.events = EPOLLIN;
    ev.data.fd = event_fd;
    
    if (epoll_ctl(epoll_fd, EPOLL_CTL_ADD, event_fd, &ev) < 0) {
        std::cerr << "[Consumer] Failed to add fd to epoll\n";
        close(epoll_fd);
        return;
    }
    
    std::cout << "[Consumer] Waiting for events...\n";
    
    int messages_received = 0;
    while (running) {
        struct epoll_event events[1];
        int nfds = epoll_wait(epoll_fd, events, 1, 1000);  // 1 sec timeout
        
        if (nfds < 0) break;
        if (nfds == 0) continue;  // Timeout
        
        // Event triggered - read all available messages
        while (true) {
            RenoirReceivedMessage msg = {};
            result = renoir_subscribe_read_next(subscriber, &msg, 0);  // 0 = non-blocking
            
            if (result == RenoirErrorCode::Success) {
                messages_received++;
                std::cout << "[Consumer] Received (event-driven): seq="
                          << msg.metadata.sequence_number
                          << ", data='" << reinterpret_cast<const char*>(msg.payload_ptr) << "'\n";
                renoir_message_release(msg.handle);
            } else if (result == RenoirErrorCode::BufferEmpty) {
                break;  // No more messages
            } else {
                std::cerr << "[Consumer] Read failed\n";
                break;
            }
        }
        
        // Clear notification
        uint64_t buf;
        ssize_t ret = read(event_fd, &buf, sizeof(buf));
        (void)ret;  // Suppress unused warning
    }
    
    close(epoll_fd);
    std::cout << "[Consumer] Received " << messages_received << " messages\n";
}
#else
void consumer_event_loop(RenoirSubscriberHandle subscriber) {
    std::cout << "[Consumer] Event mode not available on this platform, using polling\n";
    
    int messages_received = 0;
    while (running) {
        RenoirReceivedMessage msg = {};
        auto result = renoir_subscribe_read_next(subscriber, &msg, 0);  // Non-blocking
        
        if (result == RenoirErrorCode::Success) {
            messages_received++;
            std::cout << "[Consumer] Received: " 
                      << reinterpret_cast<const char*>(msg.payload_ptr) << "\n";
            renoir_message_release(msg.handle);
        } else {
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
        }
    }
    std::cout << "[Consumer] Received " << messages_received << " messages\n";
}
#endif

int main() {
    std::cout << "=== Renoir Async Topic Example ===\n";
    
    // Setup
    auto manager = renoir_topic_manager_create();
    if (!manager) return 1;
    
    RenoirTopicOptions topic_opts = {};
    topic_opts.pattern = 0;
    topic_opts.ring_capacity = 32;
    topic_opts.max_payload_size = 256;
    topic_opts.enable_notifications = true;
    
    RenoirTopicId topic_id;
    renoir_topic_register(manager, "/async/messages", &topic_opts, &topic_id);
    
    RenoirPublisherOptions pub_opts = {};
    pub_opts.batch_size = 1;
    pub_opts.timeout_ms = 100;
    
    RenoirPublisherHandle publisher = nullptr;
    renoir_publisher_create(manager, "/async/messages", &pub_opts, &publisher);
    
    RenoirSubscriberOptions sub_opts = {};
    sub_opts.mode = 0;
    sub_opts.batch_size = 1;
    sub_opts.timeout_ms = 1000;
    sub_opts.queue_depth = 32;
    
    RenoirSubscriberHandle subscriber = nullptr;
    renoir_subscriber_create(manager, "/async/messages", &sub_opts, &subscriber);
    
    std::cout << "\nStarting async pub/sub with event notifications...\n\n";
    
    // Run threads
    std::thread consumer(consumer_event_loop, subscriber);
    std::thread producer(producer_thread, publisher);
    
    // Let them run
    std::this_thread::sleep_for(std::chrono::seconds(5));
    running = false;
    
    producer.join();
    consumer.join();
    
    // Cleanup
    renoir_subscriber_destroy(subscriber);
    renoir_publisher_destroy(publisher);
    renoir_topic_manager_destroy(manager);
    
    std::cout << "\n=== Example Completed ===\n";
    return 0;
}
