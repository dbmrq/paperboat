#!/bin/bash
# Test: No orphan processes after SIGTERM
#
# Validates that when villalobos receives SIGTERM during execution,
# all child auggie processes are properly cleaned up.
#
# This test:
# 1. Starts villalobos with a task that takes ~15+ seconds
# 2. Waits ~10 seconds (mid-execution)
# 3. Sends SIGTERM to the villalobos process
# 4. Waits for cleanup
# 5. Verifies no orphan auggie processes remain

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_DIR="/tmp/villalobos_cleanup_test_$$"
PROMPT_FILE="$SCRIPT_DIR/test_prompt.txt"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Test: No Orphan Processes After SIGTERM                ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo

# Build the binary
BUILD_MODE="${1:-debug}"
if [ "$BUILD_MODE" = "release" ]; then
    BINARY="$PROJECT_ROOT/target/release/villalobos"
    cargo build --release --quiet
else
    BINARY="$PROJECT_ROOT/target/debug/villalobos"
    cargo build --quiet
fi

# Create test directory
mkdir -p "$TEST_DIR"
echo -e "${BLUE}Test directory: $TEST_DIR${NC}"

# Read the prompt (uses same test prompt as the live test)
PROMPT=$(cat "$PROMPT_FILE" | sed "s|{TEST_DIR}|$TEST_DIR|g")

# Cleanup function
cleanup() {
    # Kill villalobos process if still running
    if [ -n "$VILLALOBOS_PID" ]; then
        kill -9 "$VILLALOBOS_PID" 2>/dev/null || true
        wait "$VILLALOBOS_PID" 2>/dev/null || true
    fi
    # Clean up test directory
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

echo -e "${BLUE}Starting villalobos in background...${NC}"

# Start villalobos in background
cd "$PROJECT_ROOT"
export VILLALOBOS_SESSION_TIMEOUT=60

# Run villalobos in background, capture PID
$BINARY "$PROMPT" &
VILLALOBOS_PID=$!
echo -e "${BLUE}Villalobos started with PID: $VILLALOBOS_PID${NC}"

# Wait for agents to start (give them time to spawn auggie processes)
echo -e "${BLUE}Waiting 10 seconds for agents to start...${NC}"
sleep 10

# Check if the process is still running (it should be mid-execution)
if ! kill -0 "$VILLALOBOS_PID" 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Villalobos completed before we could send SIGTERM${NC}"
    echo -e "${YELLOW}This is okay - test task may have completed quickly${NC}"
    # Count any remaining auggie processes anyway
    AUGGIE_COUNT=$(pgrep -f "auggie.*--acp" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$AUGGIE_COUNT" -eq 0 ]; then
        echo -e "${GREEN}✓ No orphan auggie processes found${NC}"
        echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${GREEN}║  ✅ PROCESS CLEANUP TEST PASSED (early completion)         ║${NC}"
        echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
        exit 0
    fi
fi

# Record auggie process count before SIGTERM
AUGGIE_BEFORE=$(pgrep -f "auggie.*--acp" 2>/dev/null | wc -l | tr -d ' ')
echo -e "${BLUE}Auggie processes before SIGTERM: $AUGGIE_BEFORE${NC}"

echo -e "${BLUE}Sending SIGTERM to villalobos (PID: $VILLALOBOS_PID)...${NC}"
kill -TERM "$VILLALOBOS_PID" 2>/dev/null || true

# Wait for cleanup (with timeout)
echo -e "${BLUE}Waiting up to 5 seconds for cleanup...${NC}"
WAIT_COUNT=0
while kill -0 "$VILLALOBOS_PID" 2>/dev/null && [ $WAIT_COUNT -lt 10 ]; do
    sleep 0.5
    WAIT_COUNT=$((WAIT_COUNT + 1))
done

# Wait a bit more for child processes to terminate
sleep 2

# Count remaining auggie processes
AUGGIE_AFTER=$(pgrep -f "auggie.*--acp" 2>/dev/null | wc -l | tr -d ' ')
echo -e "${BLUE}Auggie processes after cleanup: $AUGGIE_AFTER${NC}"

# Verify no orphan processes
if [ "$AUGGIE_AFTER" -eq 0 ]; then
    echo
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║  ✅ PROCESS CLEANUP TEST PASSED                            ║${NC}"
    echo -e "${GREEN}║     No orphan auggie processes after SIGTERM               ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
    exit 0
else
    echo
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ PROCESS CLEANUP TEST FAILED                            ║${NC}"
    echo -e "${RED}║     Found $AUGGIE_AFTER orphan auggie process(es)                    ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    
    # Show the orphan processes for debugging
    echo -e "${YELLOW}Orphan processes:${NC}"
    pgrep -fl "auggie.*--acp" 2>/dev/null || true
    
    # Try to clean up orphan processes
    echo -e "${YELLOW}Attempting to kill orphan processes...${NC}"
    pkill -f "auggie.*--acp" 2>/dev/null || true
    
    exit 1
fi

