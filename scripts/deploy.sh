#!/usr/bin/env bash
# Deploy script for zhtw-mcp: install, uninstall, status.
#
# zhtw-mcp is a long-running MCP server managed by Claude Code.
# The running process must be killed before overwriting the binary.

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

BINARY_NAME="zhtw-mcp"

print_info()   { echo -e "${GREEN}[INFO]${NC}   $1"; }
print_warn()   { echo -e "${YELLOW}[WARN]${NC}   $1"; }
print_error()  { echo -e "${RED}[ERROR]${NC}  $1"; }
print_status() { echo -e "${BLUE}[STATUS]${NC} $1"; }

# Resolve project root relative to this script, regardless of CWD.
get_script_dir() {
    local source="${BASH_SOURCE[0]}"
    while [ -h "$source" ]; do
        local dir
        dir="$(cd -P "$(dirname "$source")" && pwd)"
        source="$(readlink "$source")"
        [[ $source != /* ]] && source="$dir/$source"
    done
    echo "$(cd -P "$(dirname "$source")" && pwd)"
}

PROJECT_ROOT="$(cd "$(get_script_dir)/.." && pwd)"

# --- helpers ------------------------------------------------------------------

detect_install_dir() {
    if [ -n "${XDG_BIN_HOME:-}" ]; then
        echo "$XDG_BIN_HOME"
    else
        echo "$HOME/.local/bin"
    fi
}

ensure_install_dir() {
    local dir="$1"
    if [ ! -d "$dir" ]; then
        print_info "Creating install directory: $dir"
        mkdir -p "$dir"
    fi
}

check_path() {
    local install_dir="${1%/}"
    case ":$PATH:" in
        *":$install_dir:"*)
            print_info "Install directory is in PATH"
            ;;
        *)
            print_warn "Install directory is not in PATH"
            echo "  Add to your shell profile: export PATH=\"$install_dir:\$PATH\""
            ;;
    esac
}

check_claude_cli() {
    if ! command -v claude &>/dev/null; then
        print_warn "Claude CLI not found in PATH"
        echo "  Install Claude Code: npm install -g @anthropic-ai/claude-code"
        return 1
    fi
    print_info "Claude CLI found: $(which claude)"
    return 0
}

# Kill running zhtw-mcp processes by exact installed binary path so we don't
# accidentally kill unrelated processes (cargo, editors, log tailers) whose
# argv happens to contain the string "zhtw-mcp".
kill_running_processes() {
    local binary_path="$1"

    # pgrep/pkill match against the full command line; anchor to the exact path.
    if pgrep -f "^${binary_path}" >/dev/null 2>&1; then
        print_info "Stopping running ${BINARY_NAME} processes..."
        pkill -f "^${binary_path}" || true
        sleep 1

        # Force kill if still alive
        if pgrep -f "^${binary_path}" >/dev/null 2>&1; then
            print_warn "Force killing ${BINARY_NAME} processes..."
            pkill -9 -f "^${binary_path}" || true
            sleep 0.5
        fi

        # Final check — installation should not proceed if kill failed
        if pgrep -f "^${binary_path}" >/dev/null 2>&1; then
            print_error "Could not stop ${BINARY_NAME} (PID: $(pgrep -f "^${binary_path}" | tr '\n' ' '))"
            echo "  Kill manually then re-run: kill \$(pgrep -f '^${binary_path}')"
            exit 1
        fi

        print_info "Stopped all ${BINARY_NAME} processes"
    fi
}

# 'claude mcp get' has no --scope flag; it searches all scopes.
# That is sufficient for existence checks — registration is still user-scoped.
mcp_server_exists() {
    if claude mcp get "$BINARY_NAME" >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

configure_mcp_server() {
    local binary_path="$1"

    if mcp_server_exists; then
        print_info "MCP server already configured (user scope)"
        return 0
    fi

    print_info "Registering MCP server with Claude Code (user scope)..."

    if claude mcp add --scope user "$BINARY_NAME" -- "$binary_path" >/dev/null 2>&1; then
        print_info "MCP server registered successfully"
    else
        print_error "Failed to register MCP server"
        echo "  Run manually: claude mcp add --scope user \"$BINARY_NAME\" -- \"$binary_path\""
        exit 1
    fi
}

remove_mcp_server() {
    if ! mcp_server_exists; then
        print_info "MCP server not configured (user scope)"
        return 0
    fi

    print_info "Removing MCP server from Claude Code (user scope)..."
    if claude mcp remove --scope user "$BINARY_NAME" >/dev/null 2>&1; then
        print_info "MCP server removed"
    else
        print_error "Failed to remove MCP server"
        echo "  Run manually: claude mcp remove --scope user \"$BINARY_NAME\""
        return 1
    fi
}

install_binary() {
    local install_dir="$1"
    local binary_src="$PROJECT_ROOT/target/release/$BINARY_NAME"

    if [ ! -f "$binary_src" ]; then
        print_error "Binary not found: $binary_src"
        echo "  Run 'make' first to build the release binary."
        exit 1
    fi

    print_info "Installing binary → $install_dir/$BINARY_NAME"
    cp "$binary_src" "$install_dir/$BINARY_NAME"
    chmod +x "$install_dir/$BINARY_NAME"
}

verify_installation() {
    local install_dir="$1"
    if [ ! -x "$install_dir/$BINARY_NAME" ]; then
        print_error "Binary installation failed or is not executable"
        exit 1
    fi
    print_info "Binary installed successfully"
}

# --- install ------------------------------------------------------------------

perform_install() {
    echo "=========================================="
    echo "  zhtw-mcp Installer"
    echo "=========================================="
    echo ""

    # Require Claude CLI upfront — registration is mandatory.
    check_claude_cli || {
        print_error "Claude CLI is required for MCP registration"
        exit 1
    }

    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/$BINARY_NAME"

    ensure_install_dir "$install_dir"

    # Must kill before overwriting — zhtw-mcp is a long-running MCP server.
    kill_running_processes "$binary_path"

    install_binary "$install_dir"
    verify_installation "$install_dir"
    check_path "$install_dir" || true
    configure_mcp_server "$binary_path"

    echo ""
    echo "=========================================="
    echo "  Installation Complete"
    echo "=========================================="
    echo ""
    echo "Binary:  $binary_path"
    echo "Claude MCP server configured (user scope)"
    echo ""
    echo "Next step: Run '/mcp' in Claude Code to connect"
    echo ""
}

# --- uninstall ----------------------------------------------------------------

perform_uninstall() {
    echo "=========================================="
    echo "  zhtw-mcp Uninstaller"
    echo "=========================================="
    echo ""

    # Support non-interactive mode via ZHTW_YES=1 or --yes flag
    local auto_yes=0
    [[ "${ZHTW_YES:-0}" == "1" ]] && auto_yes=1
    [[ "${1:-}" == "--yes" ]] && auto_yes=1

    if [[ "$auto_yes" -eq 0 ]]; then
        if [ -t 0 ]; then
            read -r -p "Are you sure you want to uninstall $BINARY_NAME? [y/N] " -n 1 REPLY
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                echo "Uninstallation cancelled"
                exit 0
            fi
        else
            print_error "Non-interactive terminal: use ZHTW_YES=1 or --yes to confirm uninstall"
            exit 1
        fi
    fi
    echo

    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/$BINARY_NAME"

    kill_running_processes "$binary_path"
    remove_mcp_server

    if [ -f "$binary_path" ]; then
        rm -f "$binary_path"
        print_info "Removed $binary_path"
    else
        print_warn "Binary not found at $binary_path"
    fi

    echo ""
    echo "=========================================="
    echo "  Uninstallation Complete"
    echo "=========================================="
    echo ""
    echo "Binary removed from: $binary_path"
    echo "MCP server configuration removed (user scope)"
    echo ""
}

# --- status -------------------------------------------------------------------

check_status() {
    local install_dir
    install_dir=$(detect_install_dir)
    local binary_path="$install_dir/$BINARY_NAME"

    print_status "Checking installation status..."
    echo ""

    if [ -x "$binary_path" ]; then
        local ver
        ver=$("$binary_path" --version 2>/dev/null || echo "unknown")
        print_info "Binary installed: $binary_path  [$ver]"
    else
        print_warn "Binary not installed at $binary_path"
    fi

    # Use the exact installed path to avoid false positives from deploy.sh itself
    # or other processes whose argv contains "zhtw-mcp".
    if pgrep -f "^${binary_path}" >/dev/null 2>&1; then
        print_info "Process is running (PID: $(pgrep -f "^${binary_path}" | tr '\n' ' '))"
    else
        print_info "Process is not running"
    fi

    check_path "$install_dir" || true

    if command -v claude &>/dev/null; then
        if mcp_server_exists; then
            print_info "Claude MCP server configured (user scope)"
        else
            print_warn "Claude MCP server not configured"
        fi
    else
        print_warn "claude CLI not found — cannot check registration"
    fi
}

# --- dispatch -----------------------------------------------------------------

case "${1:-help}" in
    install)
        perform_install
        ;;
    uninstall)
        perform_uninstall "${2:-}"
        ;;
    status)
        check_status
        ;;
    help|"")
        echo "Usage: $0 [install|uninstall [--yes]|status]"
        echo ""
        echo "  install          Kill running server, install binary, register with Claude Code."
        echo "  uninstall        Kill server, remove binary, unregister."
        echo "  uninstall --yes  Non-interactive uninstall (also: ZHTW_YES=1)."
        echo "  status           Show binary, process, and registration state."
        ;;
    *)
        print_error "Unknown command: $1"
        exit 1
        ;;
esac
