#!/bin/bash
# Live Integration Test for Villalobos Orchestrator
#
# This script runs a real integration test using actual AI agents.
# By default it uses the debug build which automatically uses Haiku
# (the cheapest/fastest model).
#
# Usage:
#   ./tests/integration/run_live_test.sh           # Uses debug build (Haiku)
#   ./tests/integration/run_live_test.sh release   # Uses release build (respects config)
#   VILLALOBOS_MODEL=sonnet4.5 ./tests/integration/run_live_test.sh  # Override model
#
# The test creates files in /tmp/villalobos_test_* which are cleaned up on success.
#
# Related tests:
#   ./tests/integration/test_process_cleanup.sh    # Tests SIGTERM handling and orphan process cleanup
#   ./tests/integration/test_completion_timing.sh  # Tests app exits promptly after completion

set -e

# Ensure child processes are killed when this script exits
cleanup() {
    if [ -n "$VILLALOBOS_PID" ]; then
        kill -- -$VILLALOBOS_PID 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_DIR="/tmp/villalobos_test_$$"
PROMPT_FILE="$SCRIPT_DIR/test_prompt.txt"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Villalobos Live Integration Test                       ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo

# Determine build mode
BUILD_MODE="${1:-debug}"
if [ "$BUILD_MODE" = "release" ]; then
    BINARY="$PROJECT_ROOT/target/release/villalobos"
    echo -e "${YELLOW}Mode: RELEASE (using configured models)${NC}"
    cargo build --release --quiet
else
    BINARY="$PROJECT_ROOT/target/debug/villalobos"
    echo -e "${YELLOW}Mode: DEBUG (using Haiku for all agents)${NC}"
    cargo build --quiet
fi

# Create test directory
mkdir -p "$TEST_DIR"
echo -e "${BLUE}Test directory: $TEST_DIR${NC}"

# Read the prompt
PROMPT=$(cat "$PROMPT_FILE" | sed "s|{TEST_DIR}|$TEST_DIR|g")

echo
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}Running orchestrator with test prompt...${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo

# Run the orchestrator with a shorter session timeout for tests
cd "$PROJECT_ROOT"
START_TIME=$(date +%s)

# Set a 60-second session timeout for tests (instead of default 30 minutes)
export VILLALOBOS_SESSION_TIMEOUT=60

if $BINARY "$PROMPT"; then
    END_TIME=$(date +%s)
    DURATION=$((END_TIME - START_TIME))
    
    echo
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}Verifying test results...${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo
    
    # Verify the expected files exist
    EXPECTED_FILES=(
        "$TEST_DIR/module_alpha.py"
        "$TEST_DIR/module_beta.py"
        "$TEST_DIR/module_gamma.py"
    )
    
    ALL_PASSED=true
    for file in "${EXPECTED_FILES[@]}"; do
        if [ -f "$file" ]; then
            echo -e "${GREEN}✓ Found: $(basename "$file")${NC}"
        else
            echo -e "${RED}✗ Missing: $(basename "$file")${NC}"
            ALL_PASSED=false
        fi
    done
    
    echo
    
    if $ALL_PASSED; then
        # Check that the test didn't take too long (should be < 90 seconds for this simple test)
        # This catches issues where we're waiting unnecessarily after work is done
        MAX_DURATION=90
        if [ "$DURATION" -gt "$MAX_DURATION" ]; then
            echo -e "${YELLOW}⚠️  WARNING: Test took ${DURATION}s (max expected: ${MAX_DURATION}s)${NC}"
            echo -e "${YELLOW}This may indicate we're waiting too long after completion${NC}"
        fi

        echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${GREEN}║  ✅ INTEGRATION TEST PASSED (${DURATION}s)                        ║${NC}"
        echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"

        # Clean up on success
        rm -rf "$TEST_DIR"
        echo -e "${BLUE}Cleaned up test directory${NC}"
        exit 0
    else
        echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${RED}║  ❌ INTEGRATION TEST FAILED - Missing expected files       ║${NC}"
        echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
        echo -e "${YELLOW}Test directory preserved: $TEST_DIR${NC}"
        echo -e "${YELLOW}Check logs/ for detailed agent output${NC}"
        exit 1
    fi
else
    echo
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ INTEGRATION TEST FAILED - Orchestrator error           ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    echo -e "${YELLOW}Test directory preserved: $TEST_DIR${NC}"
    echo -e "${YELLOW}Check logs/ for detailed agent output${NC}"
    exit 1
fi

