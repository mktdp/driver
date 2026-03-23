#!/usr/bin/env bash
#
# setup.sh — Install system dependencies for building mktdp/driver.
#
# Usage:
#   ./scripts/setup.sh
#
# Supports: Fedora, RHEL/CentOS, Ubuntu/Debian, Arch, openSUSE, Alpine.
# Run as a normal user — the script will invoke sudo when needed.

set -euo pipefail

info()  { printf '\033[1;34m[INFO]\033[0m  %s\n' "$*"; }
warn()  { printf '\033[1;33m[WARN]\033[0m  %s\n' "$*"; }
error() { printf '\033[1;31m[ERROR]\033[0m %s\n' "$*" >&2; }

# ── Detect distro ──────────────────────────────────────────────────

detect_distro() {
    if [ -f /etc/os-release ]; then
        # shellcheck disable=SC1091
        . /etc/os-release
        echo "${ID:-unknown}"
    elif command -v lsb_release &>/dev/null; then
        lsb_release -si | tr '[:upper:]' '[:lower:]'
    else
        echo "unknown"
    fi
}

DISTRO=$(detect_distro)
info "Detected distro: $DISTRO"

# ── Install packages ───────────────────────────────────────────────

case "$DISTRO" in
    fedora)
        info "Installing packages via dnf..."
        sudo dnf install -y \
            gcc gcc-c++ cmake make pkg-config \
            libusb1-devel libstdc++-static \
            systemd-devel
        ;;

    rhel|centos|rocky|alma)
        info "Installing packages via dnf..."
        sudo dnf install -y epel-release || true
        sudo dnf install -y \
            gcc gcc-c++ cmake make pkgconfig \
            libusbx-devel libstdc++-static \
            systemd-devel
        ;;

    ubuntu|debian|linuxmint|pop)
        info "Installing packages via apt..."
        sudo apt-get update
        sudo apt-get install -y \
            build-essential cmake pkg-config \
            libusb-1.0-0-dev
        ;;

    arch|manjaro|endeavouros)
        info "Installing packages via pacman..."
        sudo pacman -S --needed --noconfirm \
            gcc cmake make pkg-config \
            libusb
        ;;

    opensuse*|sles)
        info "Installing packages via zypper..."
        sudo zypper install -y \
            gcc gcc-c++ cmake make pkg-config \
            libusb-1_0-devel libstdc++-devel-static
        ;;

    alpine)
        info "Installing packages via apk..."
        sudo apk add \
            gcc g++ cmake make pkgconf \
            libusb-dev musl-dev
        ;;

    *)
        warn "Unknown distro '$DISTRO'. Please install manually:"
        echo "  - C/C++ compiler (gcc, g++)"
        echo "  - cmake, make, pkg-config"
        echo "  - libusb 1.0 development headers"
        echo "  - static libstdc++ (if on 64-bit Linux)"
        exit 1
        ;;
esac

# ── Rust toolchain ─────────────────────────────────────────────────

if command -v rustc &>/dev/null; then
    info "Rust already installed: $(rustc --version)"
else
    warn "Rust not found. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi

# ── udev rules ─────────────────────────────────────────────────────

RULES_SRC="$(cd "$(dirname "$0")/.." && pwd)/70-fingerprint.rules"
RULES_DST="/etc/udev/rules.d/70-fingerprint.rules"

if [ -f "$RULES_SRC" ]; then
    if [ ! -f "$RULES_DST" ]; then
        info "Installing udev rules for U.are.U 4500..."
        sudo cp "$RULES_SRC" "$RULES_DST"
        sudo udevadm control --reload-rules
        sudo udevadm trigger
    else
        info "udev rules already installed."
    fi
fi

# ── plugdev group ──────────────────────────────────────────────────

if ! getent group plugdev &>/dev/null; then
    info "Creating plugdev group..."
    sudo groupadd plugdev
fi

if ! id -nG "$USER" | grep -qw plugdev; then
    info "Adding $USER to plugdev group..."
    sudo usermod -aG plugdev "$USER"
    warn "Log out and back in for group change to take effect."
fi

# ── Done ───────────────────────────────────────────────────────────

info "Setup complete. Run: cargo build"
