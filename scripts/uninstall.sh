#!/bin/bash
# Virtual PLC Uninstallation Script
# Removes plc-daemon and associated files
#
# Usage:
#   sudo ./uninstall.sh
#   # or to also remove data:
#   sudo ./uninstall.sh --purge

set -euo pipefail

# Configuration
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

# Check for root/sudo
check_privileges() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root or with sudo"
        exit 1
    fi
}

# Stop and disable service
stop_service() {
    if systemctl is-active --quiet plc-daemon 2>/dev/null; then
        log_info "Stopping plc-daemon service..."
        systemctl stop plc-daemon
    fi

    if systemctl is-enabled --quiet plc-daemon 2>/dev/null; then
        log_info "Disabling plc-daemon service..."
        systemctl disable plc-daemon
    fi

    if [[ -f /etc/systemd/system/plc-daemon.service ]]; then
        log_info "Removing systemd service file..."
        rm -f /etc/systemd/system/plc-daemon.service
        systemctl daemon-reload
    fi
}

# Remove binary
remove_binary() {
    if [[ -f "${INSTALL_DIR}/plc-daemon" ]]; then
        log_info "Removing binary..."
        rm -f "${INSTALL_DIR}/plc-daemon"
    fi
}

# Remove user and group
remove_user() {
    if getent passwd "$SERVICE_USER" > /dev/null 2>&1; then
        log_info "Removing user: $SERVICE_USER"
        userdel "$SERVICE_USER" 2>/dev/null || true
    fi

    if getent group "$SERVICE_GROUP" > /dev/null 2>&1; then
        log_info "Removing group: $SERVICE_GROUP"
        groupdel "$SERVICE_GROUP" 2>/dev/null || true
    fi
}

# Remove configuration and data (with --purge)
remove_data() {
    local purge="$1"

    if [[ "$purge" == "true" ]]; then
        if [[ -d "$CONFIG_DIR" ]]; then
            log_info "Removing configuration directory: $CONFIG_DIR"
            rm -rf "$CONFIG_DIR"
        fi

        if [[ -d "$DATA_DIR" ]]; then
            log_info "Removing data directory: $DATA_DIR"
            rm -rf "$DATA_DIR"
        fi
    else
        log_warn "Configuration and data preserved in:"
        log_warn "  - $CONFIG_DIR"
        log_warn "  - $DATA_DIR"
        log_warn "Run with --purge to remove these directories"
    fi
}

# Main uninstallation
main() {
    local purge="false"

    for arg in "$@"; do
        case "$arg" in
            --purge)
                purge="true"
                ;;
            *)
                log_error "Unknown option: $arg"
                echo "Usage: $0 [--purge]"
                exit 1
                ;;
        esac
    done

    log_info "Virtual PLC Uninstaller"
    echo ""

    check_privileges

    stop_service
    remove_binary
    remove_user
    remove_data "$purge"

    echo ""
    log_info "Uninstallation complete!"
}

main "$@"
