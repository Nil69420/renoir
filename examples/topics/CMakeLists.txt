cmake_minimum_required(VERSION 3.16)

# Topic Basic Example
add_executable(topic_basic_example
    topic_basic_example.cpp
)

target_link_libraries(topic_basic_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(topic_basic_example PRIVATE cxx_std_17)

set_target_properties(topic_basic_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Topic Async Example (event-driven with epoll)
add_executable(topic_async_example
    topic_async_example.cpp
)

target_link_libraries(topic_async_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(topic_async_example PRIVATE cxx_std_17)

set_target_properties(topic_async_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Topic Comprehensive Example (topics + control region + ring buffers)
add_executable(topic_comprehensive_example
    topic_comprehensive_example.cpp
)

target_link_libraries(topic_comprehensive_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(topic_comprehensive_example PRIVATE cxx_std_17)

set_target_properties(topic_comprehensive_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Topic Peek Example (demonstrates non-destructive message inspection)
add_executable(topic_peek_example
    topic_peek_example.cpp
)

target_link_libraries(topic_peek_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(topic_peek_example PRIVATE cxx_std_17)

set_target_properties(topic_peek_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Installation
install(TARGETS 
    topic_basic_example 
    topic_async_example
    topic_comprehensive_example
    topic_peek_example
    RUNTIME DESTINATION bin
)

# Print summary
message(STATUS "Topic examples configured:")
message(STATUS "  - topic_basic_example: Basic pub/sub")
message(STATUS "  - topic_async_example: Event-driven async")
message(STATUS "  - topic_comprehensive_example: Full integration")
message(STATUS "  - topic_peek_example: Non-destructive message inspection")
