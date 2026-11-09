# C++ test suite for the Renoir wrapper layer.
# The wrapper drift guard includes renoir.hpp and exercises every wrapper
# class against the real built library, so any symbol or layout drift between
# renoir.hpp/renoir.h and the Rust FFI surface fails the build loudly.

enable_testing()

add_executable(wrapper_drift_guard wrapper_drift_guard.cpp)
target_link_libraries(wrapper_drift_guard PRIVATE renoir::cpp)
target_compile_features(wrapper_drift_guard PRIVATE cxx_std_17)

target_compile_options(wrapper_drift_guard PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Werror -fstack-protector-strong>
    $<$<CXX_COMPILER_ID:GNU,Clang>:$<$<BOOL:${ENABLE_ASAN}>:-fsanitize=address -fsanitize=undefined -fno-omit-frame-pointer>>
)

if(ENABLE_ASAN)
    target_link_options(wrapper_drift_guard PRIVATE -fsanitize=address -fsanitize=undefined)
endif()

add_test(NAME wrapper_drift_guard COMMAND wrapper_drift_guard)

add_executable(lean_cpp_test lean_cpp_test.cpp)
target_link_libraries(lean_cpp_test PRIVATE renoir::cpp)
target_compile_features(lean_cpp_test PRIVATE cxx_std_17)
target_compile_options(lean_cpp_test PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Werror -fstack-protector-strong>
)
add_test(NAME lean_cpp_test COMMAND lean_cpp_test)
