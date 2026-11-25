#!/bin/bash
# Instala Rust + target wasm + stellar CLI dentro de WSL (para compilar/desplegar el contrato Soroban)
set -e
which cc >/dev/null 2>&1 || (sudo -n apt-get update -y && sudo -n apt-get install -y build-essential) || echo "WARN: no cc"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
source "$HOME/.cargo/env"
rustup target add wasm32v1-none
if ! command -v stellar >/dev/null 2>&1; then
  URL=$(curl -s https://api.github.com/repos/stellar/stellar-cli/releases/latest | grep browser_download_url | grep 'x86_64-unknown-linux-gnu.tar.gz"' | head -1 | cut -d'"' -f4)
  echo "Descargando $URL"
  mkdir -p "$HOME/.local/bin" && curl -sSL "$URL" | tar xz -C "$HOME/.local/bin"
fi
export PATH="$HOME/.local/bin:$PATH"
cargo --version; stellar --version; which cc || true
echo DONE
