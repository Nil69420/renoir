# Examples CMakeLists.txt

# Basic C example
add_executable(renoir_c_example
    ${CMAKE_CURRENT_SOURCE_DIR}/../c_example.c
)

target_link_libraries(renoir_c_example 
    PRIVATE renoir::cpp
)

# C++ example
add_executable(renoir_cpp_example
    ${CMAKE_CURRENT_SOURCE_DIR}/../cpp_example.cpp
)

target_link_libraries(renoir_cpp_example 
    PRIVATE renoir::cpp
)

# Advanced C++ example with modern features
add_executable(renoir_modern_cpp_example
    modern_cpp_example.cpp
)

target_link_libraries(renoir_modern_cpp_example 
    PRIVATE renoir::cpp
)

target_compile_features(renoir_modern_cpp_example PRIVATE cxx_std_17)

# Performance test example (if exists)
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../performance_test.cpp")
    add_executable(renoir_performance_test
        performance_test.cpp
    )

    target_link_libraries(renoir_performance_test 
        PRIVATE renoir::cpp
    )
endif()

# Multi-process examples (if they exist)
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../multi_process_producer.cpp")
    add_executable(renoir_producer
        multi_process_producer.cpp
    )

    target_link_libraries(renoir_producer 
        PRIVATE renoir::cpp
    )
endif()

if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../multi_process_consumer.cpp")
    add_executable(renoir_consumer
        multi_process_consumer.cpp
    )

    target_link_libraries(renoir_consumer 
        PRIVATE renoir::cpp
    )
endif()

# Install examples (optional)
if(INSTALL_EXAMPLES)
    install(TARGETS 
        renoir_c_example
        renoir_cpp_example
        renoir_modern_cpp_example
        renoir_performance_test
        renoir_producer
        renoir_consumer
        DESTINATION ${CMAKE_INSTALL_BINDIR}/examples
    )
endif()