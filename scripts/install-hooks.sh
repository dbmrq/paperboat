#!/usr/bin/env bash
# Install git hooks for Paperboat development
#
# Usage: ./scripts/install-hooks.sh
#
# This script creates symlinks from .git/hooks to the scripts directory,
# making it easy to update hooks while keeping them in version control.

set -e

# Color codes
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get repository root
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$REPO_ROOT" ]; then
    echo "Error: Not in a git repository"
    exit 1
fi

HOOKS_DIR="$REPO_ROOT/.git/hooks"
SCRIPTS_DIR="$REPO_ROOT/scripts"

echo -e "${YELLOW}Installing git hooks...${NC}"

# Create hooks directory if it doesn't exist
mkdir -p "$HOOKS_DIR"

# Install pre-commit hook
if [ -f "$SCRIPTS_DIR/pre-commit" ]; then
    # Remove existing hook if it's a file (not a symlink)
    if [ -f "$HOOKS_DIR/pre-commit" ] && [ ! -L "$HOOKS_DIR/pre-commit" ]; then
        echo "Backing up existing pre-commit hook to pre-commit.bak"
        mv "$HOOKS_DIR/pre-commit" "$HOOKS_DIR/pre-commit.bak"
    fi
    
    # Create symlink (relative path for portability)
    ln -sf "../../scripts/pre-commit" "$HOOKS_DIR/pre-commit"
    chmod +x "$SCRIPTS_DIR/pre-commit"
    echo -e "${GREEN}✓ Installed pre-commit hook${NC}"
else
    echo "Warning: pre-commit script not found in $SCRIPTS_DIR"
fi

echo ""
echo -e "${GREEN}Git hooks installed successfully!${NC}"
echo ""
echo "Installed hooks:"
ls -la "$HOOKS_DIR" | grep -v "\.sample$" | grep -v "^total" | grep -v "^\.$" | grep -v "^\.\.$" || true

