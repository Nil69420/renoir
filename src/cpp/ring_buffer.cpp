/**
 * @file ring_buffer.cpp
 * @brief Ring buffer wrapper implementation
 */

#include "renoir.hpp"

namespace renoir {

RingBuffer::RingBuffer(size_t capacity) {
    checkResult(renoir_ring_buffer_create(capacity, &handle_), "create ring buffer");
}

RingBuffer::RingBuffer(RingBuffer&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
}

RingBuffer& RingBuffer::operator=(RingBuffer&& other) noexcept {
    if (this != &other) {
        if (handle_) {
            renoir_ring_buffer_destroy(handle_);
        }
        handle_ = other.handle_;
        other.handle_ = nullptr;
    }
    return *this;
}

RingBuffer::~RingBuffer() {
    if (handle_) {
        renoir_ring_buffer_destroy(handle_);
    }
}

RenoirRingStats RingBuffer::stats() const {
    RenoirRingStats raw = {};
    checkResult(renoir_ring_buffer_stats(handle_, &raw), "ring buffer stats");
    return raw;
}

bool RingBuffer::isEmpty() const {
    bool result = false;
    checkResult(renoir_ring_buffer_is_empty(handle_, &result), "ring buffer is empty");
    return result;
}

bool RingBuffer::isFull() const {
    bool result = false;
    checkResult(renoir_ring_buffer_is_full(handle_, &result), "ring buffer is full");
    return result;
}

size_t RingBuffer::availableSpace() const {
    size_t space = 0;
    checkResult(renoir_ring_buffer_available_space(handle_, &space), "ring buffer space");
    return space;
}

void RingBuffer::reset() {
    checkResult(renoir_ring_buffer_reset(handle_), "reset ring buffer");
}

void RingBuffer::push(uint8_t data) {
    RenoirProducerHandle producer = nullptr;
    checkResult(renoir_ring_buffer_producer(handle_, &producer), "ring buffer producer");
    checkResult(renoir_ring_producer_push(producer, data), "ring buffer push");
}

bool RingBuffer::tryPush(uint8_t data) {
    RenoirProducerHandle producer = nullptr;
    checkResult(renoir_ring_buffer_producer(handle_, &producer), "ring buffer producer");
    return renoir_ring_producer_try_push(producer, data) == RenoirErrorCode::RenoirErrorCode_Success;
}

uint8_t RingBuffer::pop() {
    uint8_t data = 0;
    RenoirConsumerHandle consumer = nullptr;
    checkResult(renoir_ring_buffer_consumer(handle_, &consumer), "ring buffer consumer");
    checkResult(renoir_ring_consumer_pop(consumer, &data), "ring buffer pop");
    return data;
}

bool RingBuffer::tryPop(uint8_t* data) {
    RenoirConsumerHandle consumer = nullptr;
    checkResult(renoir_ring_buffer_consumer(handle_, &consumer), "ring buffer consumer");
    return renoir_ring_consumer_try_pop(consumer, data) == RenoirErrorCode::RenoirErrorCode_Success;
}

bool RingBuffer::peek(uint8_t* data) {
    RenoirConsumerHandle consumer = nullptr;
    checkResult(renoir_ring_buffer_consumer(handle_, &consumer), "ring buffer consumer");
    return renoir_ring_consumer_peek(consumer, data) == RenoirErrorCode::RenoirErrorCode_Success;
}

} // namespace renoir
