# Build script for Renoir using CMake and Ninja

#!/bin/bash

set -e

# Configuration
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build}"
INSTALL_PREFIX="${INSTALL_PREFIX:-/usr/local}"
ENABLE_EXAMPLES="${ENABLE_EXAMPLES:-ON}"
ENABLE_TESTS="${ENABLE_TESTS:-ON}"
ENABLE_BENCHMARKS="${ENABLE_BENCHMARKS:-ON}"
WITH_RUST_BACKEND="${WITH_RUST_BACKEND:-ON}"
NINJA_JOBS="${NINJA_JOBS:-$(nproc)}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Check prerequisites
check_prerequisites() {
    print_info "Checking prerequisites..."
    
    if ! command_exists cmake; then
        print_error "CMake is not installed"
        exit 1
    fi
    
    if ! command_exists ninja; then
        print_warning "Ninja is not installed, falling back to make"
        GENERATOR="Unix Makefiles"
        PARALLEL_FLAG="-j${NINJA_JOBS}"
    else
        GENERATOR="Ninja"
        PARALLEL_FLAG="-j${NINJA_JOBS}"
    fi
    
    if [[ "${WITH_RUST_BACKEND}" == "ON" ]]; then
        if ! command_exists cargo; then
            print_error "Rust/Cargo is not installed but WITH_RUST_BACKEND=ON"
            exit 1
        fi
        print_info "Found Rust $(rustc --version)"
    fi
    
    print_info "Found CMake $(cmake --version | head -n1 | cut -d' ' -f3)"
    if command_exists ninja; then
        print_info "Found Ninja $(ninja --version)"
    fi
}

# Clean build directory
clean_build() {
    if [[ -d "${BUILD_DIR}" ]]; then
        print_info "Cleaning existing build directory..."
        rm -rf "${BUILD_DIR}"
    fi
}

# Configure project
configure_project() {
    print_info "Configuring project..."
    
    mkdir -p "${BUILD_DIR}"
    cd "${BUILD_DIR}"
    
    cmake .. \
        -G "${GENERATOR}" \
        -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
        -DCMAKE_INSTALL_PREFIX="${INSTALL_PREFIX}" \
        -DBUILD_EXAMPLES="${ENABLE_EXAMPLES}" \
        -DBUILD_TESTS="${ENABLE_TESTS}" \
        -DBUILD_BENCHMARKS="${ENABLE_BENCHMARKS}" \
        -DWITH_RUST_BACKEND="${WITH_RUST_BACKEND}" \
        -DCMAKE_EXPORT_COMPILE_COMMANDS=ON
    
    cd ..
}

# Build project
build_project() {
    print_info "Building project..."
    
    cd "${BUILD_DIR}"
    
    if [[ "${GENERATOR}" == "Ninja" ]]; then
        ninja ${PARALLEL_FLAG}
    else
        make ${PARALLEL_FLAG}
    fi
    
    cd ..
}

# Run tests
run_tests() {
    if [[ "${ENABLE_TESTS}" == "ON" ]]; then
        print_info "Running tests..."
        cd "${BUILD_DIR}"
        ctest --output-on-failure --parallel ${NINJA_JOBS}
        cd ..
    fi
}

# Install project
install_project() {
    print_info "Installing project..."
    cd "${BUILD_DIR}"
    
    if [[ "${GENERATOR}" == "Ninja" ]]; then
        ninja install
    else
        make install
    fi
    
    cd ..
}

# Package project
package_project() {
    print_info "Creating package..."
    cd "${BUILD_DIR}"
    cpack
    cd ..
}

# Build Rust components separately (for development)
build_rust_dev() {
    if [[ "${WITH_RUST_BACKEND}" == "ON" ]]; then
        print_info "Building Rust components..."
        
        if [[ "${BUILD_TYPE}" == "Debug" ]]; then
            cargo build --features c-api
        else
            cargo build --release --features c-api
        fi
        
        print_info "Running Rust tests..."
        cargo test --all-features
        
        print_info "Running Rust benchmarks..."
        cargo bench --all-features || true  # Don't fail if benchmarks fail
    fi
}

# Development build (faster iteration)
dev_build() {
    print_info "Development build mode..."
    BUILD_TYPE="Debug"
    configure_project
    build_project
    
    if [[ "${ENABLE_TESTS}" == "ON" ]]; then
        run_tests
    fi
}

# Release build (optimized)
release_build() {
    print_info "Release build mode..."
    BUILD_TYPE="Release"
    configure_project
    build_project
    
    if [[ "${ENABLE_TESTS}" == "ON" ]]; then
        run_tests
    fi
}

# Full build with packaging
full_build() {
    print_info "Full build with packaging..."
    BUILD_TYPE="Release"
    configure_project
    build_project
    run_tests
    package_project
}

# Print usage information
usage() {
    echo "Usage: $0 [OPTIONS] [COMMAND]"
    echo ""
    echo "Commands:"
    echo "  dev        Development build (Debug, faster iteration)"
    echo "  release    Release build (optimized)"
    echo "  full       Full build with packaging"
    echo "  clean      Clean build directory"
    echo "  rust       Build only Rust components"
    echo "  install    Install the project"
    echo "  test       Run tests only"
    echo ""
    echo "Options:"
    echo "  --build-dir DIR     Build directory (default: build)"
    echo "  --install-prefix    Install prefix (default: /usr/local)"
    echo "  --jobs N           Number of parallel jobs (default: $(nproc))"
    echo "  --no-rust          Disable Rust backend"
    echo "  --no-examples      Disable building examples"
    echo "  --no-tests         Disable building tests"
    echo "  --no-benchmarks    Disable building benchmarks"
    echo ""
    echo "Environment variables:"
    echo "  BUILD_TYPE         Debug or Release (default: Release)"
    echo "  BUILD_DIR          Build directory"
    echo "  INSTALL_PREFIX     Installation prefix"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --build-dir)
            BUILD_DIR="$2"
            shift 2
            ;;
        --install-prefix)
            INSTALL_PREFIX="$2"
            shift 2
            ;;
        --jobs)
            NINJA_JOBS="$2"
            shift 2
            ;;
        --no-rust)
            WITH_RUST_BACKEND="OFF"
            shift
            ;;
        --no-examples)
            ENABLE_EXAMPLES="OFF"
            shift
            ;;
        --no-tests)
            ENABLE_TESTS="OFF"
            shift
            ;;
        --no-benchmarks)
            ENABLE_BENCHMARKS="OFF"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        dev)
            COMMAND="dev"
            shift
            ;;
        release)
            COMMAND="release"
            shift
            ;;
        full)
            COMMAND="full"
            shift
            ;;
        clean)
            COMMAND="clean"
            shift
            ;;
        rust)
            COMMAND="rust"
            shift
            ;;
        install)
            COMMAND="install"
            shift
            ;;
        test)
            COMMAND="test"
            shift
            ;;
        *)
            print_error "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Default command
COMMAND="${COMMAND:-release}"

# Main execution
print_info "Renoir Build System"
print_info "=================="
print_info "Build type: ${BUILD_TYPE}"
print_info "Build directory: ${BUILD_DIR}"
print_info "Install prefix: ${INSTALL_PREFIX}"
print_info "Parallel jobs: ${NINJA_JOBS}"
print_info "Command: ${COMMAND}"
print_info ""

case "${COMMAND}" in
    clean)
        clean_build
        print_success "Build directory cleaned"
        ;;
    dev)
        check_prerequisites
        clean_build
        dev_build
        print_success "Development build completed"
        ;;
    release)
        check_prerequisites
        clean_build
        release_build
        print_success "Release build completed"
        ;;
    full)
        check_prerequisites
        clean_build
        full_build
        print_success "Full build with packaging completed"
        ;;
    rust)
        build_rust_dev
        print_success "Rust components built"
        ;;
    install)
        if [[ ! -d "${BUILD_DIR}" ]]; then
            print_error "Build directory does not exist. Run build first."
            exit 1
        fi
        install_project
        print_success "Installation completed"
        ;;
    test)
        if [[ ! -d "${BUILD_DIR}" ]]; then
            print_error "Build directory does not exist. Run build first."
            exit 1
        fi
        run_tests
        print_success "Tests completed"
        ;;
    *)
        print_error "Unknown command: ${COMMAND}"
        usage
        exit 1
        ;;
esac