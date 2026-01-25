#!/bin/bash
# Virtual PLC Installation Script
# Downloads and installs plc-daemon with systemd integration
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/hadijannat/virtual-plc/main/scripts/install.sh | bash
#   # or with specific version:
#   curl -fsSL https://raw.githubusercontent.com/hadijannat/virtual-plc/main/scripts/install.sh | bash -s -- v0.1.0

set -euo pipefail

# Configuration
REPO="hadijannat/virtual-plc"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/vplc"
DATA_DIR="/var/lib/vplc"
SERVICE_USER="plc"
SERVICE_GROUP="plc"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Detect architecture
detect_arch() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)
            echo "x86_64-unknown-linux-gnu"
            ;;
        aarch64|arm64)
            echo "aarch64-unknown-linux-gnu"
            ;;
        *)
            log_error "Unsupported architecture: $arch"
            exit 1
            ;;
    esac
}

# Get latest release version
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/'
}

# Verify we're on Linux
check_os() {
    if [[ "$(uname -s)" != "Linux" ]]; then
        log_error "This installer only supports Linux"
        exit 1
    fi
}

# Check for root/sudo
check_privileges() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root or with sudo"
        exit 1
    fi
}

# Create service user and group
create_user() {
    if ! getent group "$SERVICE_GROUP" > /dev/null 2>&1; then
        log_info "Creating group: $SERVICE_GROUP"
        groupadd -r "$SERVICE_GROUP"
    fi

    if ! getent passwd "$SERVICE_USER" > /dev/null 2>&1; then
        log_info "Creating user: $SERVICE_USER"
        useradd -r -g "$SERVICE_GROUP" -s /sbin/nologin -d "$DATA_DIR" "$SERVICE_USER"
    fi
}

# Download and verify binary
download_binary() {
    local version="$1"
    local target="$2"
    local binary_name="plc-daemon-${target}"
    local download_url="https://github.com/${REPO}/releases/download/${version}/${binary_name}"
    local checksum_url="${download_url}.sha256"

    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT

    log_info "Downloading plc-daemon ${version} for ${target}..."
    curl -fsSL "$download_url" -o "${tmp_dir}/${binary_name}"

    log_info "Downloading checksum..."
    curl -fsSL "$checksum_url" -o "${tmp_dir}/${binary_name}.sha256"

    log_info "Verifying checksum..."
    cd "$tmp_dir"
    if ! sha256sum -c "${binary_name}.sha256"; then
        log_error "Checksum verification failed!"
        exit 1
    fi

    log_info "Installing binary to ${INSTALL_DIR}/plc-daemon..."
    install -m 755 "${binary_name}" "${INSTALL_DIR}/plc-daemon"
}

# Create directories
create_directories() {
    log_info "Creating directories..."

    mkdir -p "$CONFIG_DIR"
    mkdir -p "$DATA_DIR"

    chown -R "${SERVICE_USER}:${SERVICE_GROUP}" "$DATA_DIR"
    chmod 750 "$DATA_DIR"

    chown root:${SERVICE_GROUP} "$CONFIG_DIR"
    chmod 750 "$CONFIG_DIR"
}

# Install systemd service
install_service() {
    local service_url="https://raw.githubusercontent.com/${REPO}/main/packaging/systemd/plc-daemon.service"

    log_info "Installing systemd service..."
    curl -fsSL "$service_url" -o /etc/systemd/system/plc-daemon.service

    systemctl daemon-reload

    log_info "Enabling plc-daemon service..."
    systemctl enable plc-daemon
}

# Create default config if not exists
create_default_config() {
    local version="$1"
    local config_file="${CONFIG_DIR}/config.toml"

    if [[ ! -f "$config_file" ]]; then
        log_info "Creating default configuration..."
        local config_url="https://raw.githubusercontent.com/${REPO}/${version}/config/default.toml"
        if curl -fsSL "$config_url" -o "$config_file"; then
            log_info "Downloaded default config from ${config_url}"
        else
            log_warn "Failed to download default config, writing minimal config..."
            cat > "$config_file" << 'EOF'
# Virtual PLC Configuration (minimal)
# See docs for full options

cycle_time = "10ms"
watchdog_timeout = "30ms"
max_overrun = "500us"

[metrics]
http_export = false
http_port = 8080

[fieldbus]
driver = "simulated"
EOF
        fi
        chown "${SERVICE_USER}:${SERVICE_GROUP}" "$config_file"
        chmod 640 "$config_file"
    else
        log_warn "Configuration file already exists, skipping..."
    fi
}

# Print post-install instructions
print_instructions() {
    echo ""
    log_info "Installation complete!"
    echo ""
    echo "Next steps:"
    echo "  1. Edit configuration: sudo nano ${CONFIG_DIR}/config.toml"
    echo "  2. Start the service:  sudo systemctl start plc-daemon"
    echo "  3. Check status:       sudo systemctl status plc-daemon"
    echo "  4. View logs:          sudo journalctl -u plc-daemon -f"
    echo ""
    echo "For real-time performance, ensure:"
    echo "  - PREEMPT_RT kernel is installed"
    echo "  - CPU isolation is configured in kernel parameters"
    echo "  - See: https://github.com/${REPO}#real-time-setup"
    echo ""
}

# Main installation
main() {
    local version="${1:-}"

    log_info "Virtual PLC Installer"
    echo ""

    check_os
    check_privileges

    local target
    target=$(detect_arch)
    log_info "Detected architecture: $target"

    if [[ -z "$version" ]]; then
        log_info "Fetching latest version..."
        version=$(get_latest_version)
    fi
    log_info "Version: $version"

    create_user
    download_binary "$version" "$target"
    create_directories
    install_service
    create_default_config "$version"
    print_instructions
}

main "$@"
