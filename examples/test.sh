#!/bin/bash
# Test script for Renoir C++ examples

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

EXAMPLES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$EXAMPLES_DIR/build"

echo "================================"
echo " Renoir Examples Test Suite"
echo "================================"
echo ""

# Check if build directory exists
if [ ! -d "$BUILD_DIR" ]; then
    echo -e "${RED}✗ Build directory not found${NC}"
    echo "Run ./build.sh first to build the examples"
    exit 1
fi

cd "$BUILD_DIR"

# Test counter
total_tests=0
passed_tests=0
failed_tests=0

# Function to run a test
run_test() {
    local test_name=$1
    local executable=$2
    local needs_cleanup=$3
    
    total_tests=$((total_tests + 1))
    echo -e "${YELLOW}Testing:${NC} $test_name"
    
    # Clean up shared memory if needed
    if [ "$needs_cleanup" = "true" ]; then
        rm -f /tmp/renoir_* /dev/shm/renoir_* 2>/dev/null || true
    fi
    
    # Run the test with timeout
    if timeout 10 "./$executable" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ PASSED${NC}: $test_name"
        passed_tests=$((passed_tests + 1))
        return 0
    else
        echo -e "${RED}✗ FAILED${NC}: $test_name"
        failed_tests=$((failed_tests + 1))
        return 1
    fi
}

echo "Running tests..."
echo ""

# Run all tests
run_test "Region Manager Example" "region_manager_example" "false"
run_test "Buffer Pool Example" "buffer_example" "false"
run_test "Ring Buffer Example" "ring_buffer_example" "false"
run_test "Control Region Example" "control_region_example" "true"

# Print summary
echo ""
echo "================================"
echo " Test Summary"
echo "================================"
echo "Total Tests: $total_tests"
echo -e "${GREEN}Passed: $passed_tests${NC}"
if [ $failed_tests -gt 0 ]; then
    echo -e "${RED}Failed: $failed_tests${NC}"
else
    echo "Failed: $failed_tests"
fi

# Exit with appropriate code
if [ $failed_tests -eq 0 ]; then
    echo ""
    echo -e "${GREEN}✓ All tests passed!${NC}"
    exit 0
else
    echo ""
    echo -e "${RED}✗ Some tests failed${NC}"
    exit 1
fi
