#!/bin/bash

# Renoir Shared Memory Library Build Script
# Supports Rust library building and C++ examples compilation

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BUILD_TYPE=${BUILD_TYPE:-Release}
RUST_TARGET=${RUST_TARGET:-release}
EXAMPLES_BUILD_DIR="examples/build"
NINJA_AVAILABLE=$(command -v ninja >/dev/null 2>&1 && echo "YES" || echo "NO")

print_header() {
    echo -e "${BLUE}================================${NC}"
    echo -e "${BLUE} Renoir Build System${NC}"
    echo -e "${BLUE}================================${NC}"
}

print_section() {
    echo -e "${GREEN}>>> $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}Warning: $1${NC}"
}

print_error() {
    echo -e "${RED}Error: $1${NC}"
}

show_help() {
    cat << EOF
Usage: $0 [COMMAND] [OPTIONS]

Commands:
    build-rust      Build the Rust library only
    build-examples  Build C++ examples only  
    build-all       Build Rust library and C++ examples (default)
    clean           Clean all build artifacts
    test            Run Rust tests
    check           Run Rust check (fast compilation check)
    clippy          Run Rust clippy linter
    fmt             Format Rust code
    headers         Generate C headers only
    help            Show this help message

Options:
    --debug         Build in debug mode (default: release)
    --verbose       Show verbose output
    --no-ninja      Force use of make instead of ninja

Examples:
    $0                          # Build everything in release mode
    $0 build-rust --debug       # Build Rust library in debug mode
    $0 build-examples           # Build only C++ examples
    $0 test                     # Run Rust tests
    $0 clean                    # Clean everything

EOF
}

build_rust() {
    print_section "Building Rust Library ($RUST_TARGET)"
    
    if [[ "$RUST_TARGET" == "debug" ]]; then
        cargo build ${VERBOSE:+--verbose}
    else
        cargo build --release ${VERBOSE:+--verbose}
    fi
    
    print_section "Generating C Headers"
    # Headers are generated as part of build.rs, but let's ensure they exist
    if [[ ! -f "target/include/renoir.h" ]]; then
        print_error "C headers were not generated. Check build.rs configuration."
        exit 1
    fi
    
    echo -e "${GREEN}✓ Rust library built successfully${NC}"
    echo -e "${GREEN}✓ C headers available at: target/include/renoir.h${NC}"
}

setup_cmake() {
    print_section "Setting up CMake Build System"
    
    mkdir -p "$EXAMPLES_BUILD_DIR"
    cd "$EXAMPLES_BUILD_DIR"
    
    CMAKE_GENERATOR=""
    if [[ "$NINJA_AVAILABLE" == "YES" && "$USE_MAKE" != "YES" ]]; then
        CMAKE_GENERATOR="-G Ninja"
        print_section "Using Ninja build system"
    else
        print_section "Using Make build system"
        if [[ "$USE_MAKE" == "YES" ]]; then
            print_warning "Ninja forced disabled, using Make"
        else
            print_warning "Ninja not available, falling back to Make"
        fi
    fi
    
    cmake .. $CMAKE_GENERATOR \
        -DCMAKE_BUILD_TYPE="$BUILD_TYPE" \
        -DCMAKE_EXPORT_COMPILE_COMMANDS=ON \
        ${VERBOSE:+-DCMAKE_VERBOSE_MAKEFILE=ON}
    
    cd - > /dev/null
}

build_examples() {
    print_section "Building C++ Examples ($BUILD_TYPE)"
    
    # Ensure Rust library is built first
    if [[ ! -f "target/${RUST_TARGET}/librenoir.so" && ! -f "target/${RUST_TARGET}/librenoir.a" ]]; then
        print_section "Rust library not found, building it first..."
        build_rust
    fi
    
    setup_cmake
    
    cd "$EXAMPLES_BUILD_DIR"
    if [[ "$NINJA_AVAILABLE" == "YES" && "$USE_MAKE" != "YES" ]]; then
        ninja ${VERBOSE:+-v}
    else
        make ${VERBOSE:+VERBOSE=1} -j$(nproc)
    fi
    cd - > /dev/null
    
    echo -e "${GREEN}✓ C++ examples built successfully${NC}"
    echo -e "${GREEN}✓ Executables available in: $EXAMPLES_BUILD_DIR${NC}"
}

run_tests() {
    print_section "Running Rust Tests"
    cargo test ${VERBOSE:+--verbose}
    echo -e "${GREEN}✓ All tests passed${NC}"
}

run_check() {
    print_section "Running Rust Check"
    cargo check ${VERBOSE:+--verbose}
    echo -e "${GREEN}✓ Code check passed${NC}"
}

run_clippy() {
    print_section "Running Rust Clippy"
    cargo clippy ${VERBOSE:+--verbose} -- -D warnings
    echo -e "${GREEN}✓ Clippy check passed${NC}"
}

run_fmt() {
    print_section "Formatting Rust Code"
    cargo fmt ${VERBOSE:+--verbose}
    echo -e "${GREEN}✓ Code formatted${NC}"
}

clean_all() {
    print_section "Cleaning Build Artifacts"
    
    # Clean Rust artifacts
    cargo clean
    
    # Clean C++ examples
    if [[ -d "$EXAMPLES_BUILD_DIR" ]]; then
        rm -rf "$EXAMPLES_BUILD_DIR"
        echo "Removed $EXAMPLES_BUILD_DIR"
    fi
    
    # Clean any other build artifacts
    find . -name "*.o" -delete 2>/dev/null || true
    find . -name "*.so" -delete 2>/dev/null || true
    find . -name "*.a" -delete 2>/dev/null || true
    find . -name "compile_commands.json" -delete 2>/dev/null || true
    
    echo -e "${GREEN}✓ All build artifacts cleaned${NC}"
}

generate_headers() {
    print_section "Generating C Headers Only"
    cargo build --release  # Headers are generated during build
    echo -e "${GREEN}✓ Headers generated at: target/include/renoir.h${NC}"
}

# Parse command line arguments
COMMAND=""
VERBOSE=""
USE_MAKE="NO"

while [[ $# -gt 0 ]]; do
    case $1 in
        build-rust|build-examples|build-all|clean|test|check|clippy|fmt|headers|help)
            COMMAND="$1"
            ;;
        --debug)
            BUILD_TYPE="Debug"
            RUST_TARGET="debug"
            ;;
        --verbose)
            VERBOSE="--verbose"
            ;;
        --no-ninja)
            USE_MAKE="YES"
            ;;
        *)
            print_error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
    shift
done

# Default command if none specified
if [[ -z "$COMMAND" ]]; then
    COMMAND="build-all"
fi

# Main execution
print_header

case "$COMMAND" in
    build-rust)
        build_rust
        ;;
    build-examples)
        build_examples
        ;;
    build-all)
        build_rust
        build_examples
        ;;
    clean)
        clean_all
        ;;
    test)
        run_tests
        ;;
    check)
        run_check
        ;;
    clippy)
        run_clippy
        ;;
    fmt)
        run_fmt
        ;;
    headers)
        generate_headers
        ;;
    help)
        show_help
        ;;
    *)
        print_error "Unknown command: $COMMAND"
        show_help
        exit 1
        ;;
esac

echo -e "${GREEN}✓ Build completed successfully!${NC}"