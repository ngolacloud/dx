#!/bin/sh
# DX installer — https://github.com/ngolacloud/dx
#
# Usage:
#   curl -fsSL https://ngolacloud.github.io/dx/install.sh | bash
#
# Installs the `dx` binary (crate `ngoladx-cli`) from crates.io via Cargo.
# Override the version with DX_VERSION, e.g.:
#   curl -fsSL https://ngolacloud.github.io/dx/install.sh | DX_VERSION=0.0.2 bash
#
# This script is intentionally boring: it uses Cargo (the native Rust path),
# never pipes secrets, and only touches ~/.cargo/bin.

set -eu

CRATE="ngoladx-cli"
BIN="dx"
VERSION="${DX_VERSION:-}"

say()  { printf '\033[1;36m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

say "Installing $BIN (crate $CRATE)…"

if ! command -v cargo >/dev/null 2>&1; then
  die "cargo not found. Install the Rust toolchain first:
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh
  then re-run this installer."
fi

if [ -n "$VERSION" ]; then
  cargo install "$CRATE" --version "$VERSION" --force
else
  cargo install "$CRATE" --force
fi

# Locate the installed binary. Cargo puts it in $CARGO_HOME/bin (or ~/.cargo/bin).
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
DX_PATH="$CARGO_BIN/$BIN"

if [ -x "$DX_PATH" ]; then
  say "Installed: $("$DX_PATH" --version)"
else
  warn "could not locate $BIN under $CARGO_BIN"
fi

# PATH hint.
case ":${PATH}:" in
  *":$CARGO_BIN:"*) ;;
  *) warn "$CARGO_BIN is not on your PATH. Add it:
    echo 'export PATH=\"$CARGO_BIN:\$PATH\"' >> ~/.profile" ;;
esac

# Shell autocomplete hint (dynamic: commands, flags AND values).
cat <<'EOF'

Enable shell autocomplete (commands, flags and values):

  bash   echo 'source <(COMPLETE=bash dx)'           >> ~/.bashrc
  zsh    echo 'source <(COMPLETE=zsh dx)'            >> ~/.zshrc
  fish   echo 'COMPLETE=fish dx | source'            >> ~/.config/fish/config.fish

Open a new shell (or source the file) and press TAB.
EOF

say "Done. Run '$BIN --help' to get started."
