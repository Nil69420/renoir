/**
 * @file topic_peek_example.cpp
 * @brief Example demonstrating peek functionality for non-destructive message inspection
 *
 * This example shows:
 * - Peeking at messages without consuming them
 * - Reading the same message multiple times via peek
 * - Difference between peek and consume operations
 */

#include <iostream>
#include <cstring>

extern "C" {
#include "renoir.h"
}

int main() {
    std::cout << "=== Renoir Topic Peek Example ===" << std::endl;

    std::cout << "\n1. Creating topic manager..." << std::endl;
    auto manager = renoir_topic_manager_create();
    if (manager == nullptr) {
        std::cerr << "Failed to create topic manager" << std::endl;
        return 1;
    }
    std::cout << "   ✓ Topic manager created" << std::endl;

    // Register topic
    std::cout << "\n2. Registering topic '/example/peek'..." << std::endl;
    RenoirTopicOptions topic_opts = {};
    topic_opts.pattern = 0;  // SPSC
    topic_opts.ring_capacity = 16;  // Must be power of 2
    topic_opts.max_payload_size = 1024;
    topic_opts.use_shared_pool = false;
    topic_opts.shared_pool_threshold = 0;
    topic_opts.enable_notifications = true;
    
    RenoirTopicId topic_id;
    auto result = renoir_topic_register(manager, "/example/peek", &topic_opts, &topic_id);
    if (result != 0) {
        std::cerr << "Failed to register topic: " << static_cast<int>(result) << std::endl;
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Topic registered (ID: " << topic_id << ")" << std::endl;

    // Create publisher
    std::cout << "\n3. Creating publisher..." << std::endl;
    RenoirPublisherOptions pub_opts{};
    pub_opts.batch_size = 1;
    pub_opts.timeout_ms = 100;
    pub_opts.priority = 0;
    pub_opts.enable_batching = false;
    
    RenoirPublisherHandle publisher = nullptr;
    result = renoir_publisher_create(manager, "/example/peek", &pub_opts, &publisher);
    if (result != 0) {
        std::cerr << "Failed to create publisher" << std::endl;
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Publisher created" << std::endl;

    std::cout << "\n4. Creating subscriber..." << std::endl;
    RenoirSubscriberOptions sub_opts{};
    sub_opts.mode = 0;
    sub_opts.batch_size = 1;
    sub_opts.timeout_ms = 1000;
    sub_opts.queue_depth = 16;
    sub_opts.enable_filtering = false;
    
    RenoirSubscriberHandle subscriber = nullptr;
    result = renoir_subscriber_create(manager, "/example/peek", &sub_opts, &subscriber);
    if (result != 0) {
        std::cerr << "Failed to create subscriber" << std::endl;
        renoir_publisher_destroy(publisher);
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Subscriber created" << std::endl;

    // Publish test messages
    std::cout << "\n5. Publishing test messages..." << std::endl;
    const char* messages[] = {"First", "Second", "Third"};
    for (int i = 0; i < 3; i++) {
        RenoirSequenceNumber seq = 0;
        renoir_publish(publisher, reinterpret_cast<const uint8_t*>(messages[i]),
                      strlen(messages[i]) + 1, nullptr, &seq);
        std::cout << "   Published: \"" << messages[i] << "\" (seq: " << seq << ")" << std::endl;
    }

    // Test peek functionality
    std::cout << "\n6. Testing peek functionality..." << std::endl;
    
    // Peek at first message
    std::cout << "\n   a) Peeking at first message..." << std::endl;
    RenoirReceivedMessage msg{};
    result = renoir_subscribe_peek(subscriber, &msg);
    if (result == 0) {
        std::cout << "      Peeked: \"" << reinterpret_cast<const char*>(msg.payload_ptr) << "\"" << std::endl;
        renoir_message_release(msg.handle);
    }

    // Peek again - should see same message
    std::cout << "   b) Peeking again..." << std::endl;
    result = renoir_subscribe_peek(subscriber, &msg);
    if (result == 0) {
        std::cout << "      Peeked: \"" << reinterpret_cast<const char*>(msg.payload_ptr) 
                  << "\" (same message!)" << std::endl;
        renoir_message_release(msg.handle);
    }

    // Consume the message
    std::cout << "   c) Now consuming the message..." << std::endl;
    result = renoir_subscribe_read_next(subscriber, &msg, 0);
    if (result == 0) {
        std::cout << "      Consumed: \"" << reinterpret_cast<const char*>(msg.payload_ptr) << "\"" << std::endl;
        renoir_message_release(msg.handle);
    }

    // Peek again - should see next message
    std::cout << "   d) Peeking again (should be different now)..." << std::endl;
    result = renoir_subscribe_peek(subscriber, &msg);
    if (result == 0) {
        std::cout << "      Peeked: \"" << reinterpret_cast<const char*>(msg.payload_ptr) 
                  << "\" (next message!)" << std::endl;
        renoir_message_release(msg.handle);
    }

    std::cout << "\n✓ Peek demonstration complete!" << std::endl;

    // Cleanup
    renoir_subscriber_destroy(subscriber);
    renoir_publisher_destroy(publisher);
    renoir_topic_manager_destroy(manager);

    std::cout << "\n=== Example Complete ===" << std::endl;
    return 0;
}
