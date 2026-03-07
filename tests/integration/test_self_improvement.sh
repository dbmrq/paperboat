#!/bin/bash
# Self-Improvement Integration Test for Villalobos Orchestrator
#
# This test verifies that agents can:
# 1. Discover requirements during execution (by reading a file)
# 2. Communicate discoveries via notes/add_tasks in complete()
# 3. Have the orchestrator act on those discoveries
#
# Usage:
#   ./tests/integration/test_self_improvement.sh           # Uses debug build (Haiku)
#   ./tests/integration/test_self_improvement.sh release   # Uses release build
#
# The test creates files in /tmp/villalobos_selfimprove_* which are cleaned up on success.

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
TEST_DIR="/tmp/villalobos_selfimprove_$$"
PROMPT_FILE="$SCRIPT_DIR/test_self_improvement_prompt.txt"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Villalobos Self-Improvement Integration Test          ║${NC}"
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

# Create test directory and seed files
mkdir -p "$TEST_DIR"
echo -e "${BLUE}Test directory: $TEST_DIR${NC}"

# Create empty files to be edited
touch "$TEST_DIR/file_a.py"
touch "$TEST_DIR/file_b.py"
touch "$TEST_DIR/file_c.py"

# Create the requirements.txt with the "hidden" requirement
cat > "$TEST_DIR/requirements.txt" << 'EOF'
ADDITIONAL REQUIREMENT DISCOVERED:

file_c.py MUST also export a constant: MAGIC_VALUE = 42

This requirement was not in the original task list. You should:
1. Note this in your completion message
2. Add a suggested task via add_tasks to ensure this gets done
EOF

echo -e "${BLUE}Created seed files:${NC}"
echo -e "  - file_a.py (empty)"
echo -e "  - file_b.py (empty)"
echo -e "  - file_c.py (empty)"
echo -e "  - requirements.txt (contains hidden requirement)"

# Read the prompt and substitute TEST_DIR
PROMPT=$(cat "$PROMPT_FILE" | sed "s|{TEST_DIR}|$TEST_DIR|g")

echo
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}Running orchestrator with self-improvement test prompt...${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo

# Run the orchestrator
cd "$PROJECT_ROOT"
START_TIME=$(date +%s)

# Set a longer timeout for this more complex test
export VILLALOBOS_SESSION_TIMEOUT=120

if $BINARY "$PROMPT"; then
    END_TIME=$(date +%s)
    DURATION=$((END_TIME - START_TIME))
    
    echo
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}Verifying test results...${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo
    
    ALL_PASSED=true
    
    # Check file_a.py exists and has get_a function
    if [ -f "$TEST_DIR/file_a.py" ] && grep -q "def get_a" "$TEST_DIR/file_a.py"; then
        echo -e "${GREEN}✓ file_a.py exists with get_a() function${NC}"
    else
        echo -e "${RED}✗ file_a.py missing or doesn't have get_a() function${NC}"
        ALL_PASSED=false
    fi
    
    # Check file_b.py exists and has get_b function
    if [ -f "$TEST_DIR/file_b.py" ] && grep -q "def get_b" "$TEST_DIR/file_b.py"; then
        echo -e "${GREEN}✓ file_b.py exists with get_b() function${NC}"
    else
        echo -e "${RED}✗ file_b.py missing or doesn't have get_b() function${NC}"
        ALL_PASSED=false
    fi
    
    # Check file_c.py exists and has get_c function
    if [ -f "$TEST_DIR/file_c.py" ] && grep -q "def get_c" "$TEST_DIR/file_c.py"; then
        echo -e "${GREEN}✓ file_c.py exists with get_c() function${NC}"
    else
        echo -e "${RED}✗ file_c.py missing or doesn't have get_c() function${NC}"
        ALL_PASSED=false
    fi
    
    # THE KEY TEST: Check that MAGIC_VALUE was discovered and added
    if [ -f "$TEST_DIR/file_c.py" ] && grep -q "MAGIC_VALUE.*=.*42" "$TEST_DIR/file_c.py"; then
        echo -e "${GREEN}✓ file_c.py contains MAGIC_VALUE = 42 (DISCOVERED!)${NC}"
    else
        echo -e "${RED}✗ file_c.py does NOT contain MAGIC_VALUE = 42${NC}"
        echo -e "${RED}  The self-improvement loop did not work!${NC}"
        ALL_PASSED=false
    fi
    
    echo
    
    if $ALL_PASSED; then
        echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${GREEN}║  ✅ SELF-IMPROVEMENT TEST PASSED (${DURATION}s)                    ║${NC}"
        echo -e "${GREEN}║                                                            ║${NC}"
        echo -e "${GREEN}║  Agents successfully discovered and acted on a hidden     ║${NC}"
        echo -e "${GREEN}║  requirement through the notes/add_tasks mechanism!        ║${NC}"
        echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"

        # Clean up on success
        rm -rf "$TEST_DIR"
        echo -e "${BLUE}Cleaned up test directory${NC}"
        exit 0
    else
        echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${RED}║  ❌ SELF-IMPROVEMENT TEST FAILED                           ║${NC}"
        echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
        echo
        echo -e "${YELLOW}Contents of test files:${NC}"
        for f in file_a.py file_b.py file_c.py; do
            echo -e "${YELLOW}--- $f ---${NC}"
            cat "$TEST_DIR/$f" 2>/dev/null || echo "(file not found)"
            echo
        done
        echo -e "${YELLOW}Test directory preserved: $TEST_DIR${NC}"
        echo -e "${YELLOW}Check logs/ for detailed agent output${NC}"
        exit 1
    fi
else
    echo
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ SELF-IMPROVEMENT TEST FAILED - Orchestrator error      ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    echo -e "${YELLOW}Test directory preserved: $TEST_DIR${NC}"
    echo -e "${YELLOW}Check logs/ for detailed agent output${NC}"
    exit 1
fi

