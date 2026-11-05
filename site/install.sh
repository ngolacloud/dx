#!/bin/sh
# DX installer — https://github.com/ngolacloud/dx
#
# Usage:
#   curl -fsSL https://ngolacloud.github.io/dx/install.sh | sh
#
# Installs the `dx` binary. By default it downloads a pre-compiled binary from
# the GitHub Release (no cargo/Rust required) and verifies its SHA-256. If no
# matching binary exists for your platform — or verification/download fails —
# it falls back to building from source via `cargo install` (crate ngoladx-cli).
#
# Environment overrides:
#   DX_VERSION    Pin a version, e.g. DX_VERSION=0.0.2 (tag v0.0.2). Default:
#                 latest release.
#   DX_BIN_DIR    Install directory for the downloaded binary.
#                 Default: ~/.local/bin
#   DX_METHOD     Force the install path: "binary" or "cargo".
#
# This script is intentionally boring: it never pipes secrets and only writes
# to the install directory (or ~/.cargo/bin on the cargo fallback).

set -eu

REPO="ngolacloud/dx"
CRATE="ngoladx-cli"
BIN="dx"
VERSION="${DX_VERSION:-}"
BIN_DIR="${DX_BIN_DIR:-$HOME/.local/bin}"
METHOD="${DX_METHOD:-auto}"

say()  { printf '\033[1;36m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# --- platform detection -----------------------------------------------------
detect_asset() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)  os_tag="linux" ;;
    Darwin) os_tag="macos" ;;
    *)      return 1 ;;
  esac
  case "$arch" in
    x86_64|amd64)        arch_tag="x86_64" ;;
    aarch64|arm64)       arch_tag="aarch64" ;;
    *)                   return 1 ;;
  esac
  printf '%s-%s-%s' "$BIN" "$os_tag" "$arch_tag"
}

# --- downloader -------------------------------------------------------------
download() { # url dest
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    return 2
  fi
}

sha256_of() { # file -> prints hash
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    return 3
  fi
}

# --- binary install path ----------------------------------------------------
install_binary() {
  asset="$1"
  if [ -n "$VERSION" ]; then
    base="https://github.com/$REPO/releases/download/v$VERSION"
  else
    base="https://github.com/$REPO/releases/latest/download"
  fi

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  say "Downloading $asset from $base …"
  download "$base/$asset" "$tmp/$asset" || return 1
  download "$base/$asset.sha256" "$tmp/$asset.sha256" || {
    warn "no checksum published for $asset"
    return 1
  }

  want="$(awk '{print $1}' "$tmp/$asset.sha256")"
  got="$(sha256_of "$tmp/$asset")" || {
    warn "no sha256 tool (sha256sum/shasum) available to verify the download"
    return 1
  }
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    die "checksum mismatch for $asset
    expected: $want
    actual:   $got"
  fi
  say "SHA-256 verified."

  mkdir -p "$BIN_DIR"
  install -m 0755 "$tmp/$asset" "$BIN_DIR/$BIN" 2>/dev/null \
    || { cp "$tmp/$asset" "$BIN_DIR/$BIN" && chmod 0755 "$BIN_DIR/$BIN"; }

  say "Installed: $("$BIN_DIR/$BIN" --version)"
  path_hint "$BIN_DIR"
  autocomplete_hint
  say "Done. Run '$BIN --help' to get started."
}

# --- cargo fallback ---------------------------------------------------------
install_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    die "no pre-compiled binary available and cargo not found.
    Install the Rust toolchain first:
      curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh
    then re-run this installer (or pass DX_METHOD=cargo)."
  fi
  say "Building $BIN from source via cargo (crate $CRATE)…"
  if [ -n "$VERSION" ]; then
    cargo install "$CRATE" --version "$VERSION" --force
  else
    cargo install "$CRATE" --force
  fi
  CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
  if [ -x "$CARGO_BIN/$BIN" ]; then
    say "Installed: $("$CARGO_BIN/$BIN" --version)"
  else
    warn "could not locate $BIN under $CARGO_BIN"
  fi
  path_hint "$CARGO_BIN"
  autocomplete_hint
  say "Done. Run '$BIN --help' to get started."
}

# --- hints ------------------------------------------------------------------
path_hint() { # dir
  case ":${PATH}:" in
    *":$1:"*) ;;
    *) warn "$1 is not on your PATH. Add it:
    echo 'export PATH=\"$1:\$PATH\"' >> ~/.profile" ;;
  esac
}

autocomplete_hint() {
  cat <<'EOF'

Enable shell autocomplete (commands, flags and values):

  bash   echo 'source <(COMPLETE=bash dx)'           >> ~/.bashrc
  zsh    echo 'source <(COMPLETE=zsh dx)'            >> ~/.zshrc
  fish   echo 'COMPLETE=fish dx | source'            >> ~/.config/fish/config.fish

Open a new shell (or source the file) and press TAB.
EOF
}

# --- main -------------------------------------------------------------------
say "Installing $BIN…"

if [ "$METHOD" = "cargo" ]; then
  install_cargo
  exit 0
fi

asset="$(detect_asset || true)"
if [ "$METHOD" = "binary" ]; then
  [ -n "$asset" ] || die "no pre-compiled binary for $(uname -s)/$(uname -m)."
  install_binary "$asset"
  exit 0
fi

# auto: try binary, fall back to cargo.
if [ -n "$asset" ] && install_binary "$asset"; then
  exit 0
fi
warn "falling back to building from source…"
install_cargo
