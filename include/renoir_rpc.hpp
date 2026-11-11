#ifndef RENOIR_RPC_HPP
#define RENOIR_RPC_HPP

#include "renoir.hpp"

#include <cstddef>
#include <cstdint>
#include <string>

namespace renoir {

class RpcClient {
public:
    static constexpr size_t kMaxPayloadBytes = 488;

    RpcClient(const std::string& service, uint32_t maxPayload, uint32_t requestSlots = 64) {
        checkResult(renoir_rpc_client_open(service.c_str(), maxPayload, requestSlots, &handle_),
                    "rpc client open");
    }

    RpcClient(const RpcClient&) = delete;
    RpcClient& operator=(const RpcClient&) = delete;

    RpcClient(RpcClient&& other) noexcept : handle_(other.handle_) { other.handle_ = nullptr; }

    RpcClient& operator=(RpcClient&& other) noexcept {
        if (this != &other) {
            renoir_rpc_client_close(handle_);
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~RpcClient() { renoir_rpc_client_close(handle_); }

    void setStampRequests(bool stamp) noexcept { renoir_rpc_client_set_stamp(handle_, stamp ? 1 : 0); }
    int serversAlive() noexcept { return renoir_rpc_client_servers_alive(handle_); }
    bool waitForServer(uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_rpc_client_wait_for_server(handle_, timeoutNs) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    RenoirErrorCode call(const void* request, size_t requestLen, void* response,
                         size_t responseCapacity, size_t& responseLen,
                         uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_rpc_call(handle_, static_cast<const uint8_t*>(request), requestLen,
                               static_cast<uint8_t*>(response), responseCapacity, timeoutNs,
                               &responseLen);
    }

private:
    RenoirRpcHandle handle_ = nullptr;
};

struct RpcRequestInfo {
    uint32_t clientId = 0;
    uint64_t sequence = 0;
    size_t length = 0;
};

class RpcServer {
public:
    RpcServer(const std::string& service, uint32_t maxPayload, uint32_t requestSlots = 64) {
        checkResult(renoir_rpc_server_open(service.c_str(), maxPayload, requestSlots, &handle_),
                    "rpc server open");
    }

    RpcServer(const RpcServer&) = delete;
    RpcServer& operator=(const RpcServer&) = delete;

    RpcServer(RpcServer&& other) noexcept : handle_(other.handle_) { other.handle_ = nullptr; }

    RpcServer& operator=(RpcServer&& other) noexcept {
        if (this != &other) {
            renoir_rpc_server_close(handle_);
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~RpcServer() { renoir_rpc_server_close(handle_); }

    void setRequestLifespan(uint64_t lifespanNs) noexcept {
        renoir_rpc_server_set_lifespan(handle_, lifespanNs);
    }
    uint64_t staleRequests() const noexcept { return renoir_rpc_server_stale_requests(handle_); }

    bool poll(void* buffer, size_t capacity, RpcRequestInfo& info) noexcept {
        return renoir_rpc_poll(handle_, static_cast<uint8_t*>(buffer), capacity, &info.clientId,
                               &info.sequence, &info.length) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    bool waitRequest(uint32_t spinIterations = 2000, uint64_t timeoutNs = UINT64_MAX) noexcept {
        return renoir_rpc_wait_request(handle_, spinIterations, timeoutNs) ==
               RenoirErrorCode::RenoirErrorCode_Success;
    }

    RenoirErrorCode reply(const RpcRequestInfo& info, const void* data, size_t len) noexcept {
        return renoir_rpc_reply(handle_, info.clientId, info.sequence,
                                static_cast<const uint8_t*>(data), len);
    }

private:
    RenoirRpcHandle handle_ = nullptr;
};

}  // namespace renoir

#endif
