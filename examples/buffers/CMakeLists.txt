cmake_minimum_required(VERSION 3.16)

# Buffer Management Example
add_executable(buffer_example
    buffer_example.cpp
)

target_link_libraries(buffer_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(buffer_example PRIVATE cxx_std_17)

# Set output directory
set_target_properties(buffer_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Ring Buffer Example
add_executable(ring_buffer_example
    ring_buffer_example.cpp
)

target_link_libraries(ring_buffer_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(ring_buffer_example PRIVATE cxx_std_17)

set_target_properties(ring_buffer_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Control Region Example
add_executable(control_region_example
    control_region_example.cpp
)

target_link_libraries(control_region_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(control_region_example PRIVATE cxx_std_17)

set_target_properties(control_region_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Add custom targets for running examples
add_custom_target(run_buffer_example
    COMMAND buffer_example
    DEPENDS buffer_example
    WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    COMMENT "Running buffer management example"
)

add_custom_target(run_ring_buffer_example
    COMMAND ring_buffer_example
    DEPENDS ring_buffer_example
    WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    COMMENT "Running ring buffer example"
)

add_custom_target(run_control_region_example
    COMMAND control_region_example
    DEPENDS control_region_example
    WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    COMMENT "Running control region example"
)