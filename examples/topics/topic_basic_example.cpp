/**
 * @file topic_basic_example.cpp
 * @brief Basic topic pub/sub example
 * 
 * This example shows fundamental topic operations:
 * - Creating a topic manager
 * - Registering a topic
 * - Creating publisher and subscriber
 * - Publishing and receiving messages
 * - Getting statistics
 */

#include <iostream>
#include <cstring>
#include <chrono>

extern "C" {
#include "renoir.h"
}

int main() {
    std::cout << "=== Renoir Topic Basic Example ===" << std::endl;
    
    // Create topic manager
    std::cout << "\n1. Creating topic manager..." << std::endl;
    RenoirTopicManagerHandle manager = renoir_topic_manager_create();
    if (manager == nullptr) {
        std::cerr << "Failed to create topic manager" << std::endl;
        return 1;
    }
    std::cout << "   ✓ Topic manager created" << std::endl;
    
    // Register topic
    std::cout << "\n2. Registering topic '/example/messages'..." << std::endl;
    RenoirTopicOptions topic_opts = {};
    topic_opts.pattern = 0;  // SPSC
    topic_opts.ring_capacity = 16;  // Must be power of 2
    topic_opts.max_payload_size = 1024;
    topic_opts.use_shared_pool = false;
    topic_opts.shared_pool_threshold = 0;
    topic_opts.enable_notifications = true;
    
    RenoirTopicId topic_id;
    auto result = renoir_topic_register(manager, "/example/messages", &topic_opts, &topic_id);
    if (result != RenoirErrorCode::Success) {
        std::cerr << "Failed to register topic: " << static_cast<int>(result) << std::endl;
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Topic registered (ID: " << topic_id << ")" << std::endl;
    
    // Create publisher
    std::cout << "\n3. Creating publisher..." << std::endl;
    RenoirPublisherOptions pub_opts = {};
    pub_opts.batch_size = 1;
    pub_opts.timeout_ms = 100;
    pub_opts.priority = 0;
    pub_opts.enable_batching = false;
    
    RenoirPublisherHandle publisher = nullptr;
    result = renoir_publisher_create(manager, "/example/messages", &pub_opts, &publisher);
    if (result != RenoirErrorCode::Success || publisher == nullptr) {
        std::cerr << "Failed to create publisher" << std::endl;
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Publisher created" << std::endl;
    
    // Create subscriber
    std::cout << "\n4. Creating subscriber..." << std::endl;
    RenoirSubscriberOptions sub_opts = {};
    sub_opts.mode = 0;
    sub_opts.batch_size = 1;
    sub_opts.timeout_ms = 1000;
    sub_opts.queue_depth = 16;
    sub_opts.enable_filtering = false;
    
    RenoirSubscriberHandle subscriber = nullptr;
    result = renoir_subscriber_create(manager, "/example/messages", &sub_opts, &subscriber);
    if (result != RenoirErrorCode::Success || subscriber == nullptr) {
        std::cerr << "Failed to create subscriber" << std::endl;
        renoir_publisher_destroy(publisher);
        renoir_topic_manager_destroy(manager);
        return 1;
    }
    std::cout << "   ✓ Subscriber created" << std::endl;
    
    // Publish messages
    std::cout << "\n5. Publishing 5 messages..." << std::endl;
    for (int i = 0; i < 5; i++) {
        char message_data[256];
        snprintf(message_data, sizeof(message_data), "Test message #%d", i);
        
        RenoirMessageMetadata metadata = {};
        metadata.timestamp_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now().time_since_epoch()
        ).count();
        metadata.sequence_number = i;
        metadata.source_id = 1;
        metadata.message_type = 0;
        metadata.flags = 0;
        
        RenoirSequenceNumber seq_out = 0;
        result = renoir_publish(
            publisher,
            reinterpret_cast<const uint8_t*>(message_data),
            strlen(message_data) + 1,
            &metadata,
            &seq_out
        );
        
        if (result == RenoirErrorCode::Success) {
            std::cout << "   ✓ Published [seq=" << seq_out << "]: " << message_data << std::endl;
        }
    }
    
    // Read messages
    std::cout << "\n6. Reading messages..." << std::endl;
    for (int i = 0; i < 5; i++) {
        RenoirReceivedMessage msg = {};
        result = renoir_subscribe_read_next(subscriber, &msg, 1000);
        
        if (result == RenoirErrorCode::Success) {
            std::cout << "   ✓ Received [seq=" << msg.metadata.sequence_number << "]: '"
                      << reinterpret_cast<const char*>(msg.payload_ptr) << "'" << std::endl;
            renoir_message_release(msg.handle);
        } else if (result == RenoirErrorCode::BufferEmpty) {
            std::cout << "   (timeout)" << std::endl;
            break;
        }
    }
    
    // Statistics
    std::cout << "\n7. Statistics:" << std::endl;
    RenoirTopicStats stats = {};
    result = renoir_topic_stats(manager, "/example/messages", &stats);
    if (result == RenoirErrorCode::Success) {
        std::cout << "   Published: " << stats.messages_published << std::endl;
        std::cout << "   Consumed:  " << stats.messages_consumed << std::endl;
        std::cout << "   Dropped:   " << stats.messages_dropped << std::endl;
    }
    
    // Cleanup
    std::cout << "\n8. Cleanup..." << std::endl;
    renoir_subscriber_destroy(subscriber);
    renoir_publisher_destroy(publisher);
    renoir_topic_manager_destroy(manager);
    std::cout << "   ✓ All resources cleaned up" << std::endl;
    
    std::cout << "\n=== Example Completed Successfully! ===" << std::endl;
    return 0;
}
