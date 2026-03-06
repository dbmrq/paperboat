#!/bin/bash
# Unified Integration Test Runner for Villalobos
#
# This script automatically discovers and runs all integration tests.
# Tests are discovered by finding executable files matching test_*.sh pattern.
#
# Usage:
#   ./tests/integration/run_all_tests.sh           # Run all tests (debug build)
#   ./tests/integration/run_all_tests.sh release   # Run all tests (release build)
#   ./tests/integration/run_all_tests.sh --list    # List available tests without running
#
# Test Discovery:
#   - Looks for executable test_*.sh files in tests/integration/
#   - Each test must exit 0 on success, non-zero on failure
#   - Tests are run sequentially in alphabetical order
#
# Adding New Tests:
#   1. Create a new file: tests/integration/test_<name>.sh
#   2. Make it executable: chmod +x tests/integration/test_<name>.sh
#   3. Ensure it exits 0 on success, non-zero on failure
#   4. It will be automatically discovered and run

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Parse arguments
BUILD_MODE="debug"
LIST_ONLY=false

for arg in "$@"; do
    case $arg in
        release)
            BUILD_MODE="release"
            ;;
        --list)
            LIST_ONLY=true
            ;;
    esac
done

# Discover all test scripts
discover_tests() {
    # Use -perm to check for executable (works on both macOS and Linux)
    find "$SCRIPT_DIR" -maxdepth 1 -name "test_*.sh" -type f -perm +111 2>/dev/null | sort
}

# List available tests
list_tests() {
    echo -e "${BLUE}Available integration tests:${NC}"
    echo
    local tests=$(discover_tests)
    if [ -z "$tests" ]; then
        echo -e "${YELLOW}  No tests found${NC}"
        return
    fi
    for test in $tests; do
        local name=$(basename "$test")
        # Extract description from file header (first comment line after shebang)
        local desc=$(sed -n '2s/^# *//p' "$test" | head -1)
        echo -e "  ${GREEN}$name${NC}"
        if [ -n "$desc" ]; then
            echo -e "    ${BLUE}$desc${NC}"
        fi
    done
}

if $LIST_ONLY; then
    list_tests
    exit 0
fi

echo -e "${BOLD}${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${BLUE}║     Villalobos Integration Test Suite                      ║${NC}"
echo -e "${BOLD}${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo
echo -e "${BLUE}Build mode: ${BOLD}$BUILD_MODE${NC}"
echo

# Discover tests
TESTS=$(discover_tests)
TEST_COUNT=$(echo "$TESTS" | grep -c . || echo 0)

if [ "$TEST_COUNT" -eq 0 ]; then
    echo -e "${YELLOW}No integration tests found in $SCRIPT_DIR${NC}"
    exit 0
fi

echo -e "${BLUE}Discovered $TEST_COUNT test(s):${NC}"
for test in $TESTS; do
    echo -e "  - $(basename "$test")"
done
echo

# Track results
PASSED=0
FAILED=0
FAILED_TESTS=""
START_TIME=$(date +%s)

# Run each test
for test in $TESTS; do
    TEST_NAME=$(basename "$test")
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}Running: ${BOLD}$TEST_NAME${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo
    
    TEST_START=$(date +%s)
    
    # Run the test, passing build mode
    if "$test" "$BUILD_MODE"; then
        TEST_END=$(date +%s)
        TEST_DURATION=$((TEST_END - TEST_START))
        echo
        echo -e "${GREEN}✓ $TEST_NAME passed (${TEST_DURATION}s)${NC}"
        PASSED=$((PASSED + 1))
    else
        TEST_END=$(date +%s)
        TEST_DURATION=$((TEST_END - TEST_START))
        echo
        echo -e "${RED}✗ $TEST_NAME failed (${TEST_DURATION}s)${NC}"
        FAILED=$((FAILED + 1))
        FAILED_TESTS="$FAILED_TESTS  - $TEST_NAME\n"
    fi
    echo
done

END_TIME=$(date +%s)
TOTAL_DURATION=$((END_TIME - START_TIME))

# Summary
echo -e "${BOLD}${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BOLD}${BLUE}                        TEST SUMMARY                           ${NC}"
echo -e "${BOLD}${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo
echo -e "  Total tests: $TEST_COUNT"
echo -e "  ${GREEN}Passed: $PASSED${NC}"
echo -e "  ${RED}Failed: $FAILED${NC}"
echo -e "  Duration: ${TOTAL_DURATION}s"
echo

if [ "$FAILED" -eq 0 ]; then
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║  ✅ ALL TESTS PASSED                                       ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
    exit 0
else
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ SOME TESTS FAILED                                      ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    echo
    echo -e "${RED}Failed tests:${NC}"
    echo -e "$FAILED_TESTS"
    exit 1
fi

