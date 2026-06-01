#!/usr/bin/env sh
# install.sh — k7s one-line installer
#
# Downloads the pre-built k7s binary for the current OS and architecture from
# the latest GitHub Release (or a pinned version) and installs it to
# /usr/local/bin (falling back to ~/bin if root access is unavailable).
#
# Usage:
#   # Latest release (default):
#   curl -fsSL https://raw.githubusercontent.com/neuro-ng/k7s/main/install.sh | sh
#
#   # Pin a specific version:
#   curl -fsSL https://raw.githubusercontent.com/neuro-ng/k7s/main/install.sh | sh -s -- --version v0.1.1
#
#   # Install to a custom directory:
#   curl -fsSL https://raw.githubusercontent.com/neuro-ng/k7s/main/install.sh | sh -s -- --install-dir ~/.local/bin
#
# Options:
#   --version <tag>       Version tag to install (default: latest)
#   --install-dir <path>  Target installation directory (default: /usr/local/bin)
#   --no-completions      Skip installing shell completions
#   --dry-run             Print what would be done without downloading anything

set -eu

REPO="neuro-ng/k7s"
BINARY="k7s"
VERSION=""
INSTALL_DIR=""
SKIP_COMPLETIONS=0
DRY_RUN=0

# ── Argument parsing ─────────────────────────────────────────────────────────

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            VERSION="$2"; shift 2 ;;
        --version=*)
            VERSION="${1#*=}"; shift ;;
        --install-dir)
            INSTALL_DIR="$2"; shift 2 ;;
        --install-dir=*)
            INSTALL_DIR="${1#*=}"; shift ;;
        --no-completions)
            SKIP_COMPLETIONS=1; shift ;;
        --dry-run)
            DRY_RUN=1; shift ;;
        -h|--help)
            sed -n '/^# Usage:/,/^[^#]/{ /^[^#]/d; s/^# \{0,2\}//; p }' "$0"
            exit 0 ;;
        *)
            echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ── Helpers ──────────────────────────────────────────────────────────────────

info()  { printf '\033[1;34m==> \033[0m%s\n' "$*" >&2; }
ok()    { printf '\033[1;32m  ✓ \033[0m%s\n' "$*" >&2; }
warn()  { printf '\033[1;33m  ! \033[0m%s\n' "$*" >&2; }
die()   { printf '\033[1;31merror: \033[0m%s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "Required tool not found: $1 — please install it and retry."
}

# ── Detect OS + arch ─────────────────────────────────────────────────────────

detect_target() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64)  echo "x86_64-unknown-linux-musl" ;;
                aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
                *) die "Unsupported Linux architecture: $ARCH" ;;
            esac ;;
        Darwin)
            case "$ARCH" in
                x86_64)  echo "x86_64-apple-darwin" ;;
                arm64)   echo "aarch64-apple-darwin" ;;
                *) die "Unsupported macOS architecture: $ARCH" ;;
            esac ;;
        *)
            die "Unsupported operating system: $OS (supported: Linux, macOS)" ;;
    esac
}

# ── Resolve latest version via GitHub API ────────────────────────────────────

resolve_version() {
    need curl

    info "Resolving latest k7s release..."
    API_URL="https://api.github.com/repos/${REPO}/releases/latest"

    RESOLVED="$(curl -fsSL "$API_URL" \
        -H "Accept: application/vnd.github+json" \
        | grep '"tag_name"' \
        | head -1 \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"

    [ -n "$RESOLVED" ] || die "Could not resolve latest release. Check your network or pass --version manually."
    echo "$RESOLVED"
}

# ── Determine install directory ───────────────────────────────────────────────

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        echo "$INSTALL_DIR"
        return
    fi

    # Prefer /usr/local/bin if writable (or can sudo), else ~/bin
    if [ -w /usr/local/bin ]; then
        echo "/usr/local/bin"
    elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
        echo "/usr/local/bin"
    else
        echo "$HOME/bin"
    fi
}

needs_sudo() {
    DIR="$1"
    [ ! -w "$DIR" ] && command -v sudo >/dev/null 2>&1
}

# ── Main ─────────────────────────────────────────────────────────────────────

main() {
    need curl
    need tar

    TARGET="$(detect_target)"
    ok "Detected target: $TARGET"

    if [ -z "$VERSION" ]; then
        VERSION="$(resolve_version)"
    fi
    ok "Version: $VERSION"

    ARCHIVE="${BINARY}-${VERSION}-${TARGET}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
    INSTALL_DIR="$(resolve_install_dir)"

    info "Downloading: $DOWNLOAD_URL"

    if [ "$DRY_RUN" -eq 1 ]; then
        echo ""
        echo "  [dry-run] Would download: $DOWNLOAD_URL"
        echo "  [dry-run] Would install:  ${INSTALL_DIR}/${BINARY}"
        if [ "$SKIP_COMPLETIONS" -eq 0 ]; then
            echo "  [dry-run] Would install completions from archive"
        fi
        exit 0
    fi

    # Create a temp directory for extraction
    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

    curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "${TMP_DIR}/${ARCHIVE}" \
        || die "Download failed. Check that version '${VERSION}' exists at:
  https://github.com/${REPO}/releases"

    info "Extracting archive..."
    tar xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"

    EXTRACTED_BIN="${TMP_DIR}/${BINARY}"
    [ -f "$EXTRACTED_BIN" ] || die "Binary '${BINARY}' not found in archive."
    chmod 0755 "$EXTRACTED_BIN"

    # ── Install binary ────────────────────────────────────────────────────────

    mkdir -p "$INSTALL_DIR"
    info "Installing ${BINARY} → ${INSTALL_DIR}/${BINARY}"

    if needs_sudo "$INSTALL_DIR"; then
        sudo mv "$EXTRACTED_BIN" "${INSTALL_DIR}/${BINARY}"
    else
        mv "$EXTRACTED_BIN" "${INSTALL_DIR}/${BINARY}"
    fi

    ok "Installed: ${INSTALL_DIR}/${BINARY}"

    # ── Install shell completions ─────────────────────────────────────────────

    if [ "$SKIP_COMPLETIONS" -eq 0 ] && [ -d "${TMP_DIR}/completions" ]; then
        install_completions "${TMP_DIR}/completions"
    fi

    # ── PATH check ───────────────────────────────────────────────────────────

    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            warn "${INSTALL_DIR} is not in your PATH."
            echo "  Add it to your shell profile:"
            echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac

    echo ""
    ok "k7s ${VERSION} installed successfully!"
    echo ""
    echo "  Run: ${BINARY} --help"
    echo ""
}

# ── Shell completions ─────────────────────────────────────────────────────────

install_completions() {
    COMP_DIR="$1"

    SHELL_NAME="$(basename "${SHELL:-}")"

    case "$SHELL_NAME" in
        bash)
            BASH_COMP_DIRS="
                $HOME/.local/share/bash-completion/completions
                /etc/bash_completion.d
                /usr/local/etc/bash_completion.d"
            for d in $BASH_COMP_DIRS; do
                if [ -d "$d" ] && [ -w "$d" ]; then
                    cp "${COMP_DIR}/k7s.bash" "$d/k7s" 2>/dev/null && \
                        ok "Bash completions → $d/k7s" && return
                fi
            done
            warn "Could not install bash completions automatically."
            echo "  Copy manually: cp ${COMP_DIR}/k7s.bash ~/.local/share/bash-completion/completions/k7s"
            ;;
        zsh)
            ZSH_COMP_DIRS="
                $HOME/.zsh/completions
                /usr/local/share/zsh/site-functions
                /usr/share/zsh/site-functions"
            for d in $ZSH_COMP_DIRS; do
                if [ -d "$d" ] && [ -w "$d" ]; then
                    cp "${COMP_DIR}/k7s.zsh" "$d/_k7s" 2>/dev/null && \
                        ok "Zsh completions → $d/_k7s" && return
                fi
            done
            # Fallback: create dir
            mkdir -p "$HOME/.zsh/completions"
            cp "${COMP_DIR}/k7s.zsh" "$HOME/.zsh/completions/_k7s" 2>/dev/null && \
                ok "Zsh completions → $HOME/.zsh/completions/_k7s"
            echo "  Add to ~/.zshrc if not already present:"
            echo "    fpath=(\$HOME/.zsh/completions \$fpath)"
            ;;
        fish)
            FISH_COMP_DIR="$HOME/.config/fish/completions"
            mkdir -p "$FISH_COMP_DIR"
            cp "${COMP_DIR}/k7s.fish" "$FISH_COMP_DIR/k7s.fish" 2>/dev/null && \
                ok "Fish completions → $FISH_COMP_DIR/k7s.fish"
            ;;
        *)
            warn "Unknown shell '$SHELL_NAME' — skipping completion install."
            echo "  Completions are in the downloaded archive under completions/"
            ;;
    esac
}

main "$@"
