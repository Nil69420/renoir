/**
 * @file renoir_wrapper.cpp
 * @brief Topic manager, publisher and subscriber wrapper implementation
 */

#include "renoir.hpp"

namespace renoir {

RenoirTopicOptions TopicOptions::toC() const {
    RenoirTopicOptions c = {};
    c.pattern = pattern;
    c.ring_capacity = ring_capacity;
    c.max_payload_size = max_payload_size;
    c.use_shared_pool = use_shared_pool;
    c.shared_pool_threshold = shared_pool_threshold;
    c.enable_notifications = enable_notifications;
    return c;
}

void ReceivedMessage::release() {
    if (handle_) {
        renoir_message_release(handle_);
        handle_ = nullptr;
    }
}

TopicManager::TopicManager() {
    handle_ = renoir_topic_manager_create();
    if (!handle_) {
        throw Exception(RenoirErrorCode::RenoirErrorCode_OutOfMemory, "create topic manager");
    }
}

TopicManager::TopicManager(TopicManager&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
}

TopicManager& TopicManager::operator=(TopicManager&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_topic_manager_destroy(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

TopicManager::~TopicManager() {
    if (handle_) {
        renoir_topic_manager_destroy(handle_);
    }
}

RenoirTopicId TopicManager::registerTopic(const std::string& name, const TopicOptions& options) {
    RenoirTopicOptions c = options.toC();
    RenoirTopicId topic_id = 0;
    checkResult(renoir_topic_register(handle_, name.c_str(), &c, &topic_id),
                "register topic");
    return topic_id;
}

RenoirTopicId TopicManager::lookupTopic(const std::string& name) const {
    RenoirTopicId topic_id = 0;
    checkResult(renoir_topic_lookup(handle_, name.c_str(), &topic_id), "lookup topic");
    return topic_id;
}

void TopicManager::removeTopic(const std::string& name) {
    checkResult(renoir_topic_remove(handle_, name.c_str()), "remove topic");
}

size_t TopicManager::topicCount() const {
    return renoir_topic_count(handle_);
}

std::vector<RenoirTopicInfo> TopicManager::listTopics() const {
    RenoirTopicInfo* topics = nullptr;
    size_t count = 0;
    checkResult(renoir_topic_list(handle_, &topics, &count), "list topics");
    std::vector<RenoirTopicInfo> result(topics, topics + count);
    renoir_topic_list_free(topics, count);
    return result;
}

Publisher TopicManager::createPublisher(const std::string& topic_name) {
    RenoirPublisherOptions opts = {};
    opts.batch_size = 1;
    opts.timeout_ms = 0;
    RenoirPublisherHandle publisher = nullptr;
    checkResult(renoir_publisher_create(handle_, topic_name.c_str(), &opts, &publisher),
                "create publisher");
    return Publisher(publisher);
}

Subscriber TopicManager::createSubscriber(const std::string& topic_name) {
    RenoirSubscriberOptions opts = {};
    opts.mode = 0;
    opts.batch_size = 1;
    opts.timeout_ms = 1000;
    RenoirSubscriberHandle subscriber = nullptr;
    checkResult(renoir_subscriber_create(handle_, topic_name.c_str(), &opts, &subscriber),
                "create subscriber");
    return Subscriber(subscriber);
}

RenoirTopicStats TopicManager::topicStats(const std::string& topic_name) const {
    RenoirTopicStats stats = {};
    checkResult(renoir_topic_stats(handle_, topic_name.c_str(), &stats), "topic stats");
    return stats;
}

Publisher::Publisher(Publisher&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
}

Publisher& Publisher::operator=(Publisher&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_publisher_destroy(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

Publisher::~Publisher() {
    if (handle_) {
        renoir_publisher_destroy(handle_);
    }
}

void Publisher::publish(const std::vector<uint8_t>& payload, RenoirSequenceNumber* sequence) {
    RenoirSequenceNumber seq = 0;
    checkResult(renoir_publish(handle_, payload.data(), payload.size(), nullptr, &seq),
                "publish message");
    if (sequence) {
        *sequence = seq;
    }
}

bool Publisher::tryPublish(const std::vector<uint8_t>& payload, RenoirSequenceNumber* sequence) {
    RenoirSequenceNumber seq = 0;
    RenoirErrorCode result =
        renoir_publish_try(handle_, payload.data(), payload.size(), nullptr, &seq);
    if (result == RenoirErrorCode::RenoirErrorCode_BufferFull) {
        return false;
    }
    checkResult(result, "try publish message");
    if (sequence) {
        *sequence = seq;
    }
    return true;
}

bool Publisher::hasSubscribers() const {
    bool result = false;
    checkResult(renoir_publisher_has_subscribers(handle_, &result), "publisher has subscribers");
    return result;
}

RenoirTopicStats Publisher::stats() const {
    RenoirTopicStats stats = {};
    checkResult(renoir_publisher_stats(handle_, &stats), "publisher stats");
    return stats;
}

std::string Publisher::topicName() const {
    const char* raw = renoir_publisher_topic_name(handle_);
    if (!raw) {
        return std::string();
    }
    std::string result(raw);
    renoir_free_string(const_cast<char*>(raw));
    return result;
}

Subscriber::Subscriber(Subscriber&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
}

Subscriber& Subscriber::operator=(Subscriber&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_subscriber_destroy(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

Subscriber::~Subscriber() {
    if (handle_) {
        renoir_subscriber_destroy(handle_);
    }
}

ReceivedMessage Subscriber::readNext(uint64_t timeout_ms) {
    RenoirReceivedMessage msg = {};
    checkResult(renoir_subscribe_read_next(handle_, &msg, timeout_ms), "read next message");
    return ReceivedMessage(msg, msg.handle);
}

bool Subscriber::peek(ReceivedMessage* message) {
    RenoirReceivedMessage msg = {};
    RenoirErrorCode result = renoir_subscribe_peek(handle_, &msg);
    if (result == RenoirErrorCode::RenoirErrorCode_BufferEmpty) {
        return false;
    }
    checkResult(result, "peek message");
    *message = ReceivedMessage(msg, msg.handle);
    return true;
}

size_t Subscriber::drain() {
    size_t count = 0;
    checkResult(renoir_subscribe_drain(handle_, &count), "drain messages");
    return count;
}

bool Subscriber::hasMessages() const {
    bool result = false;
    checkResult(renoir_subscriber_has_messages(handle_, &result), "subscriber has messages");
    return result;
}

size_t Subscriber::pendingCount() const {
    size_t count = 0;
    checkResult(renoir_subscriber_pending_count(handle_, &count), "pending message count");
    return count;
}

RenoirTopicStats Subscriber::stats() const {
    RenoirTopicStats stats = {};
    checkResult(renoir_subscriber_stats(handle_, &stats), "subscriber stats");
    return stats;
}

std::string Subscriber::topicName() const {
    const char* raw = renoir_subscriber_topic_name(handle_);
    if (!raw) {
        return std::string();
    }
    std::string result(raw);
    renoir_free_string(const_cast<char*>(raw));
    return result;
}

int Subscriber::eventFd() const {
    int fd = -1;
    checkResult(renoir_subscriber_get_event_fd(handle_, &fd), "get subscriber event fd");
    return fd;
}

std::string getVersionString() {
    char* raw = renoir_version_string();
    if (!raw) {
        return "unknown";
    }
    std::string result(raw);
    renoir_free_string(raw);
    return result;
}

} // namespace renoir
