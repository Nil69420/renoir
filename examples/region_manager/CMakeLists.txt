cmake_minimum_required(VERSION 3.16)

# Region Manager Example
add_executable(region_manager_example
    region_manager_example.cpp
)

target_link_libraries(region_manager_example
    PRIVATE
    renoir
    Threads::Threads
)

target_compile_features(region_manager_example PRIVATE cxx_std_17)

# Set output directory
set_target_properties(region_manager_example PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
)

# Add custom target for running the example
add_custom_target(run_region_manager
    COMMAND region_manager_example
    DEPENDS region_manager_example
    WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    COMMENT "Running region manager example"
)