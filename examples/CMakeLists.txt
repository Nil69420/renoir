cmake_minimum_required(VERSION 3.16)
project(RenoirExamples 
    VERSION 0.1.0
    DESCRIPTION "C++ Examples for Renoir Shared Memory Library"
    LANGUAGES CXX)

# Set C++ standard
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Build options
option(BUILD_SHARED_LIBS "Build shared libraries" ON)
option(ENABLE_EXAMPLES "Build examples" ON)

# Compiler flags
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

set(CMAKE_CXX_FLAGS_DEBUG "-g -O0 -DDEBUG")
set(CMAKE_CXX_FLAGS_RELEASE "-O3 -DNDEBUG")

# Find required packages
find_package(PkgConfig REQUIRED)
find_package(Threads REQUIRED)

# Set paths for the Renoir library
set(RENOIR_ROOT_DIR "${CMAKE_CURRENT_SOURCE_DIR}/..")
set(RENOIR_INCLUDE_DIR "${RENOIR_ROOT_DIR}/target/include")
set(RENOIR_LIB_DIR "${RENOIR_ROOT_DIR}/target/release")

# Check if Renoir library exists
if(NOT EXISTS "${RENOIR_INCLUDE_DIR}/renoir.h")
    message(FATAL_ERROR 
        "Renoir C headers not found at ${RENOIR_INCLUDE_DIR}/renoir.h. "
        "Please build the Rust library first using './build.sh build-rust'")
endif()

if(NOT EXISTS "${RENOIR_LIB_DIR}/librenoir.so" AND NOT EXISTS "${RENOIR_LIB_DIR}/librenoir.a")
    message(FATAL_ERROR 
        "Renoir library not found in ${RENOIR_LIB_DIR}. "
        "Please build the Rust library first using './build.sh build-rust'")
endif()

# Create imported target for Renoir library
add_library(renoir SHARED IMPORTED)
set_target_properties(renoir PROPERTIES
    IMPORTED_LOCATION "${RENOIR_LIB_DIR}/librenoir.so"
    INTERFACE_INCLUDE_DIRECTORIES "${RENOIR_INCLUDE_DIR}"
)

# Common compile options for all examples
add_compile_options(
    -Wall
    -Wextra
    -Wpedantic
    -Wno-unused-parameter
    $<$<CONFIG:Debug>:-fsanitize=address>
    $<$<CONFIG:Debug>:-fsanitize=undefined>
)

# Common link options
add_link_options(
    $<$<CONFIG:Debug>:-fsanitize=address>
    $<$<CONFIG:Debug>:-fsanitize=undefined>
)

# Include directories
include_directories(${RENOIR_INCLUDE_DIR})

# Add example subdirectories
if(ENABLE_EXAMPLES)
    add_subdirectory(region_manager)
    add_subdirectory(buffers)
endif()

# Print configuration summary
message(STATUS "=== Renoir Examples Configuration ===")
message(STATUS "Build Type: ${CMAKE_BUILD_TYPE}")
message(STATUS "C++ Standard: ${CMAKE_CXX_STANDARD}")
message(STATUS "Renoir Include Dir: ${RENOIR_INCLUDE_DIR}")
message(STATUS "Renoir Library Dir: ${RENOIR_LIB_DIR}")
message(STATUS "Examples Enabled: ${ENABLE_EXAMPLES}")
message(STATUS "======================================")