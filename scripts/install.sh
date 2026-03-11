#!/bin/sh
# Paperboat installer for Unix systems (macOS, Linux)
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/dbmrq/paperboat/main/scripts/install.sh | sh
#
# Options (via environment variables):
#   PAPERBOAT_INSTALL_DIR  - Installation directory (default: /usr/local/bin or ~/.local/bin)
#   PAPERBOAT_VERSION      - Specific version to install (default: latest)
#   PAPERBOAT_BINARY_PATH  - Path to local binary (for testing, skips download)
#   PAPERBOAT_NO_HOMEBREW  - Set to 1 to skip Homebrew even if available on macOS
#
# This script will:
# 1. On macOS with Homebrew: use `brew install` for automatic updates
# 2. Otherwise: download the appropriate binary from GitHub releases

REPO="dbmrq/paperboat"
BINARY_NAME="paperboat"

# Colors (if terminal supports them)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

info() { printf "${BLUE}info:${NC} %s\n" "$1"; }
warn() { printf "${YELLOW}warn:${NC} %s\n" "$1"; }
success() { printf "${GREEN}✓${NC} %s\n" "$1"; }

# Error handler that waits for user input before exiting
error() {
    printf "${RED}error:${NC} %s\n" "$1" >&2
    printf "\n"
    printf "${YELLOW}Installation failed. Press Enter to exit...${NC}"
    read -r _
    exit 1
}

# Trap to handle unexpected errors and show output before terminal closes
# Note: ERR trap is not POSIX but is supported by bash/zsh which is what macOS uses
handle_error() {
    _exit_code=$1
    _line_number=$2
    printf "\n${RED}error:${NC} Installation failed at line %s with exit code %s\n" "$_line_number" "$_exit_code" >&2
    printf "\n"
    printf "${YELLOW}Press Enter to exit...${NC}"
    read -r _
    exit "$_exit_code"
}

# Set up error trap (works in bash/zsh, ignored in pure POSIX sh)
trap 'handle_error $? $LINENO' ERR 2>/dev/null || true

set -e

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Darwin) echo "Darwin" ;;
        Linux)  echo "Linux" ;;
        MINGW*|MSYS*|CYGWIN*) error "Windows detected. Use the PowerShell installer instead:
  irm https://raw.githubusercontent.com/dbmrq/paperboat/main/scripts/install.ps1 | iex" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64) echo "x86_64" ;;
        arm64|aarch64) echo "arm64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Get latest version from GitHub API
get_latest_version() {
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/'
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/'
    else
        error "curl or wget is required"
    fi
}

# Download file
download() {
    url="$1"
    output="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "$url" -o "$output"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$output"
    else
        error "curl or wget is required"
    fi
}

# Determine install directory
get_install_dir() {
    if [ -n "${PAPERBOAT_INSTALL_DIR:-}" ]; then
        echo "$PAPERBOAT_INSTALL_DIR"
    elif [ -w "/usr/local/bin" ]; then
        echo "/usr/local/bin"
    else
        mkdir -p "$HOME/.local/bin"
        echo "$HOME/.local/bin"
    fi
}

# Check if Homebrew should be used
should_use_homebrew() {
    # Skip if explicitly disabled or using local binary for testing
    [ -n "${PAPERBOAT_NO_HOMEBREW:-}" ] && return 1
    [ -n "${PAPERBOAT_BINARY_PATH:-}" ] && return 1

    # Only on macOS
    [ "$(uname -s)" != "Darwin" ] && return 1

    # Check if brew is available
    command -v brew >/dev/null 2>&1
}

install_with_homebrew() {
    info "Using Homebrew for installation (enables automatic updates)"

    # Temporarily disable set -e so we can capture brew output
    set +e

    # Add tap if not already added
    if ! brew tap | grep -q "dbmrq/tap"; then
        info "Adding tap dbmrq/tap..."
        BREW_OUTPUT=$(brew tap dbmrq/tap 2>&1)
        BREW_EXIT_CODE=$?
        if [ $BREW_EXIT_CODE -ne 0 ]; then
            printf "\n%s\n" "$BREW_OUTPUT"
            error "Failed to add Homebrew tap (exit code $BREW_EXIT_CODE)"
        fi
    fi

    # Install or upgrade
    if brew list paperboat >/dev/null 2>&1; then
        info "Upgrading paperboat..."
        BREW_OUTPUT=$(brew upgrade paperboat 2>&1)
        BREW_EXIT_CODE=$?
    else
        info "Installing paperboat..."
        BREW_OUTPUT=$(brew install dbmrq/tap/paperboat 2>&1)
        BREW_EXIT_CODE=$?
    fi

    # Re-enable set -e
    set -e

    if [ $BREW_EXIT_CODE -ne 0 ]; then
        printf "\n%s\n" "$BREW_OUTPUT"
        error "Homebrew command failed (exit code $BREW_EXIT_CODE)"
    fi

    # Show brew output on success too (in case of warnings)
    if [ -n "$BREW_OUTPUT" ]; then
        printf "%s\n" "$BREW_OUTPUT"
    fi

    success "Paperboat installed successfully via Homebrew!"
    echo ""
    echo "To update in the future, run:"
    echo "  brew upgrade paperboat"
    echo ""
    echo "Get started with:"
    echo "  paperboat --help"
}

install_binary() {
    OS=$(detect_os)
    ARCH=$(detect_arch)
    info "Detected: $OS $ARCH"

    VERSION="${PAPERBOAT_VERSION:-$(get_latest_version)}"
    if [ -z "$VERSION" ]; then
        error "Could not determine latest version"
    fi
    info "Version: $VERSION"

    INSTALL_DIR=$(get_install_dir)
    info "Install directory: $INSTALL_DIR"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    if [ -n "${PAPERBOAT_BINARY_PATH:-}" ]; then
        # Use local binary (for testing)
        info "Using local binary: $PAPERBOAT_BINARY_PATH"
        cp "$PAPERBOAT_BINARY_PATH" "$TMP_DIR/$BINARY_NAME"
    else
        # Download from GitHub releases
        ARCHIVE_NAME="${BINARY_NAME}_${VERSION}_${OS}_${ARCH}.tar.gz"
        DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE_NAME}"

        info "Downloading $ARCHIVE_NAME..."
        download "$DOWNLOAD_URL" "$TMP_DIR/$ARCHIVE_NAME"

        info "Extracting..."
        tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$TMP_DIR"
    fi

    info "Installing to $INSTALL_DIR..."
    mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    success "Paperboat $VERSION installed successfully!"

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            echo "Add this to your shell profile:"
            echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
            ;;
    esac

    echo ""
    echo "Get started with:"
    echo "  paperboat --help"
}

main() {
    info "Installing Paperboat..."

    if should_use_homebrew; then
        install_with_homebrew
    else
        install_binary
    fi
}

main "$@"

