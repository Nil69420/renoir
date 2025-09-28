cmake_minimum_required(VERSION 3.16)
project(renoir VERSION 0.1.0 LANGUAGES C CXX)

# Set C++ standard
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Set build type if not specified
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

# Options
option(BUILD_SHARED_LIBS "Build shared libraries" ON)
option(BUILD_EXAMPLES "Build example programs" ON)
option(BUILD_TESTS "Build tests" ON)
option(BUILD_BENCHMARKS "Build benchmarks" ON)
option(WITH_RUST_BACKEND "Build with Rust backend library" ON)
option(ENABLE_MEMFD "Enable memfd support (Linux only)" ON)

# Find required packages
find_package(Threads REQUIRED)

# Detect target architecture and OS
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    set(RENOIR_PLATFORM_LINUX ON)
    if(ENABLE_MEMFD)
        add_definitions(-DRENOIR_HAS_MEMFD=1)
    endif()
elseif(CMAKE_SYSTEM_NAME STREQUAL "Windows")
    set(RENOIR_PLATFORM_WINDOWS ON)
elseif(CMAKE_SYSTEM_NAME STREQUAL "Darwin")
    set(RENOIR_PLATFORM_MACOS ON)
endif()

# Rust configuration
if(WITH_RUST_BACKEND)
    # Find Cargo
    find_program(CARGO_EXECUTABLE cargo REQUIRED)
    
    # Determine Rust target directory
    if(CMAKE_BUILD_TYPE STREQUAL "Debug")
        set(RUST_BUILD_TYPE "debug")
        set(RUST_TARGET_DIR "${CMAKE_CURRENT_SOURCE_DIR}/target/debug")
    else()
        set(RUST_BUILD_TYPE "release")
        set(RUST_TARGET_DIR "${CMAKE_CURRENT_SOURCE_DIR}/target/release")
    endif()
    
    # Rust library names
    if(WIN32)
        set(RUST_LIB_NAME "renoir.dll")
        set(RUST_STATIC_LIB_NAME "renoir.lib")
    elseif(APPLE)
        set(RUST_LIB_NAME "librenoir.dylib")
        set(RUST_STATIC_LIB_NAME "librenoir.a")
    else()
        set(RUST_LIB_NAME "librenoir.so")
        set(RUST_STATIC_LIB_NAME "librenoir.a")
    endif()
    
    # Rust build command
    if(CMAKE_BUILD_TYPE STREQUAL "Debug")
        set(CARGO_BUILD_ARGS --features c-api)
    else()
        set(CARGO_BUILD_ARGS --release --features c-api)
    endif()
    
    # Custom target to build Rust library
    add_custom_target(rust_build
        COMMAND ${CARGO_EXECUTABLE} build ${CARGO_BUILD_ARGS}
        WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
        COMMENT "Building Rust library"
        VERBATIM
    )
    
    # Create imported library target
    add_library(renoir_rust SHARED IMPORTED GLOBAL)
    add_dependencies(renoir_rust rust_build)
    
    set_target_properties(renoir_rust PROPERTIES
        IMPORTED_LOCATION "${RUST_TARGET_DIR}/${RUST_LIB_NAME}"
        INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_SOURCE_DIR}/include"
    )
    
    # Create static library target as well
    add_library(renoir_rust_static STATIC IMPORTED GLOBAL)
    add_dependencies(renoir_rust_static rust_build)
    
    set_target_properties(renoir_rust_static PROPERTIES
        IMPORTED_LOCATION "${RUST_TARGET_DIR}/${RUST_STATIC_LIB_NAME}"
        INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_SOURCE_DIR}/include"
    )
endif()

# Create C++ wrapper library
add_library(renoir_cpp
    src/cpp/renoir_wrapper.cpp
    src/cpp/memory_region.cpp
    src/cpp/buffer_pool.cpp
    src/cpp/ring_buffer.cpp
)

target_include_directories(renoir_cpp
    PUBLIC
        $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
        $<INSTALL_INTERFACE:include>
    PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/src/cpp
)

target_compile_features(renoir_cpp PUBLIC cxx_std_17)

# Link with Rust library if enabled
if(WITH_RUST_BACKEND)
    if(BUILD_SHARED_LIBS)
        target_link_libraries(renoir_cpp PUBLIC renoir_rust)
    else()
        target_link_libraries(renoir_cpp PUBLIC renoir_rust_static)
    endif()
endif()

# Platform-specific libraries
if(RENOIR_PLATFORM_LINUX)
    target_link_libraries(renoir_cpp PRIVATE rt pthread)
elseif(RENOIR_PLATFORM_WINDOWS)
    target_link_libraries(renoir_cpp PRIVATE ws2_32)
endif()

# Compiler-specific options
target_compile_options(renoir_cpp PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic>
    $<$<CXX_COMPILER_ID:MSVC>:/W4>
    $<$<CONFIG:Release>:-O3>
    $<$<CONFIG:Debug>:-g -O0>
)

# Add alias for consistent naming
add_library(renoir::cpp ALIAS renoir_cpp)

# Examples
if(BUILD_EXAMPLES)
    add_subdirectory(examples/cmake)
endif()

# Tests
if(BUILD_TESTS AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/tests/cmake")
    enable_testing()
    add_subdirectory(tests/cmake)
endif()

# Benchmarks
if(BUILD_BENCHMARKS AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/benchmarks/cmake")
    add_subdirectory(benchmarks/cmake)
endif()

# Installation
include(GNUInstallDirs)

# Install headers
install(
    FILES include/renoir.h include/renoir.hpp
    DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)

# Install libraries
install(
    TARGETS renoir_cpp
    EXPORT RenoirTargets
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)

# Install Rust library if built
if(WITH_RUST_BACKEND)
    install(
        FILES "${RUST_TARGET_DIR}/${RUST_LIB_NAME}"
        DESTINATION ${CMAKE_INSTALL_LIBDIR}
        OPTIONAL
    )
    
    if(NOT BUILD_SHARED_LIBS)
        install(
            FILES "${RUST_TARGET_DIR}/${RUST_STATIC_LIB_NAME}"
            DESTINATION ${CMAKE_INSTALL_LIBDIR}
            OPTIONAL
        )
    endif()
endif()

# Export targets
install(
    EXPORT RenoirTargets
    FILE RenoirTargets.cmake
    NAMESPACE renoir::
    DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/renoir
)

# Create config file
include(CMakePackageConfigHelpers)

configure_package_config_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/cmake/RenoirConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/RenoirConfig.cmake"
    INSTALL_DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/renoir
)

write_basic_package_version_file(
    "${CMAKE_CURRENT_BINARY_DIR}/RenoirConfigVersion.cmake"
    VERSION ${PROJECT_VERSION}
    COMPATIBILITY AnyNewerVersion
)

install(
    FILES
        "${CMAKE_CURRENT_BINARY_DIR}/RenoirConfig.cmake"
        "${CMAKE_CURRENT_BINARY_DIR}/RenoirConfigVersion.cmake"
    DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/renoir
)

# Uninstall target
if(NOT TARGET uninstall)
    configure_file(
        "${CMAKE_CURRENT_SOURCE_DIR}/cmake/cmake_uninstall.cmake.in"
        "${CMAKE_CURRENT_BINARY_DIR}/cmake_uninstall.cmake"
        IMMEDIATE @ONLY
    )
    
    add_custom_target(uninstall
        COMMAND ${CMAKE_COMMAND} -P ${CMAKE_CURRENT_BINARY_DIR}/cmake_uninstall.cmake
    )
endif()

# Print configuration summary
message(STATUS "")
message(STATUS "Renoir Configuration Summary:")
message(STATUS "  Version: ${PROJECT_VERSION}")
message(STATUS "  Build type: ${CMAKE_BUILD_TYPE}")
message(STATUS "  C++ Standard: ${CMAKE_CXX_STANDARD}")
message(STATUS "  Shared libs: ${BUILD_SHARED_LIBS}")
message(STATUS "  Rust backend: ${WITH_RUST_BACKEND}")
message(STATUS "  Examples: ${BUILD_EXAMPLES}")
message(STATUS "  Tests: ${BUILD_TESTS}")
message(STATUS "  Benchmarks: ${BUILD_BENCHMARKS}")
if(RENOIR_PLATFORM_LINUX AND ENABLE_MEMFD)
message(STATUS "  Memfd support: ON")
else()
message(STATUS "  Memfd support: OFF")
endif()
message(STATUS "  Install prefix: ${CMAKE_INSTALL_PREFIX}")
message(STATUS "")