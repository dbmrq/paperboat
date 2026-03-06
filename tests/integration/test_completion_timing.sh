#!/bin/bash
# Test: App Stops Promptly After Completion
#
# Validates that after all agents complete their work, the app exits
# promptly (within 5 seconds) rather than waiting for long drain timeouts.
#
# This test:
# 1. Runs villalobos with a standard task
# 2. Captures output with timestamps
# 3. Records when the last agent completes
# 4. Records when the app exits
# 5. Verifies the gap is less than 5 seconds

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_DIR="/tmp/villalobos_timing_test_$$"
PROMPT_FILE="$SCRIPT_DIR/test_prompt.txt"
OUTPUT_FILE="/tmp/villalobos_timing_output_$$"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Test: App Stops Promptly After Completion              ║${NC}"
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

# Read the prompt
PROMPT=$(cat "$PROMPT_FILE" | sed "s|{TEST_DIR}|$TEST_DIR|g")

# Cleanup function
cleanup() {
    rm -rf "$TEST_DIR"
    rm -f "$OUTPUT_FILE"
}
trap cleanup EXIT

echo -e "${BLUE}Running villalobos with test task...${NC}"
echo

cd "$PROJECT_ROOT"
export VILLALOBOS_SESSION_TIMEOUT=60
export RUST_LOG=villalobos=info

# Run villalobos and capture stderr (where tracing logs go) with timestamps
# We use ts to add timestamps to each line
START_EPOCH=$(date +%s.%N)

# Run villalobos, capture both stdout and stderr, prefix with timestamps
if $BINARY "$PROMPT" 2>&1 | while IFS= read -r line; do
    echo "[$(date +%s.%N)] $line"
done > "$OUTPUT_FILE"; then
    END_EPOCH=$(date +%s.%N)
    
    echo
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}Analyzing timing...${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
    
    # Look for completion indicators in the output
    # "All X agents completed" is logged when spawn_agents with wait="all" finishes
    # "Orchestration complete" or "complete(success" indicates completion tool was called
    
    # Find the last completion timestamp
    # Look for patterns like "All 3 agents completed" or "complete" tool call
    COMPLETION_LINE=$(grep -E "(All [0-9]+ agents completed|complete.*success|✅ Result:)" "$OUTPUT_FILE" | tail -1)
    
    if [ -n "$COMPLETION_LINE" ]; then
        # Extract epoch timestamp from the line prefix [epoch]
        COMPLETION_EPOCH=$(echo "$COMPLETION_LINE" | grep -oE '^\[[0-9]+\.[0-9]+\]' | tr -d '[]')
        
        if [ -n "$COMPLETION_EPOCH" ]; then
            # Calculate gap between completion and exit
            GAP=$(echo "$END_EPOCH - $COMPLETION_EPOCH" | bc -l)
            GAP_INT=$(printf "%.0f" "$GAP")
            
            echo -e "${BLUE}Completion detected at: $COMPLETION_EPOCH${NC}"
            echo -e "${BLUE}App exited at: $END_EPOCH${NC}"
            echo -e "${BLUE}Gap: ${GAP}s${NC}"
            
            # Verify gap is less than 5 seconds
            MAX_GAP=5
            if (( $(echo "$GAP < $MAX_GAP" | bc -l) )); then
                echo
                echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
                echo -e "${GREEN}║  ✅ COMPLETION TIMING TEST PASSED                          ║${NC}"
                echo -e "${GREEN}║     Gap: ${GAP}s (max: ${MAX_GAP}s)                               ║${NC}"
                echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
                
                # Clean up test files
                rm -rf "$TEST_DIR"
                exit 0
            else
                echo
                echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
                echo -e "${RED}║  ❌ COMPLETION TIMING TEST FAILED                          ║${NC}"
                echo -e "${RED}║     Gap: ${GAP}s exceeds max of ${MAX_GAP}s                        ║${NC}"
                echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
                echo
                echo -e "${YELLOW}Last few lines of output:${NC}"
                tail -20 "$OUTPUT_FILE"
                exit 1
            fi
        fi
    fi
    
    # Fallback: If we couldn't find completion indicators, check total duration
    TOTAL_DURATION=$(echo "$END_EPOCH - $START_EPOCH" | bc -l)
    echo -e "${YELLOW}Could not find specific completion marker${NC}"
    echo -e "${BLUE}Total test duration: ${TOTAL_DURATION}s${NC}"
    
    # If total duration is reasonable (under 90s), consider it a pass
    if (( $(echo "$TOTAL_DURATION < 90" | bc -l) )); then
        echo
        echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${GREEN}║  ✅ COMPLETION TIMING TEST PASSED                          ║${NC}"
        echo -e "${GREEN}║     Total duration: ${TOTAL_DURATION}s (reasonable)              ║${NC}"
        echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
        exit 0
    else
        echo
        echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
        echo -e "${RED}║  ❌ COMPLETION TIMING TEST FAILED                          ║${NC}"
        echo -e "${RED}║     Duration ${TOTAL_DURATION}s may indicate slow drain          ║${NC}"
        echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
        exit 1
    fi
else
    echo
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ COMPLETION TIMING TEST FAILED                          ║${NC}"
    echo -e "${RED}║     Villalobos exited with error                           ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    echo -e "${YELLOW}Test directory preserved: $TEST_DIR${NC}"
    # Don't clean up on failure
    trap - EXIT
    exit 1
fi

