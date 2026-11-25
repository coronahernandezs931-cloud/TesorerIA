#!/bin/bash
# Compila y prueba el contrato dentro de WSL (copia a disco Linux por velocidad)
set -e
source ~/.cargo/env; export PATH=$HOME/.local/bin:$PATH
SRC=/mnt/c/Users/coron/Downloads/ProyGoya/tesoreria/contracts
mkdir -p ~/tesoreria-contracts && cp -r $SRC/Cargo.toml $SRC/treasury-guard ~/tesoreria-contracts/
cd ~/tesoreria-contracts
cargo test --quiet 2>&1 | tail -25
stellar contract build --quiet 2>&1 | tail -5 || stellar contract build
mkdir -p $SRC/../state && cp target/wasm32v1-none/release/treasury_guard.wasm $SRC/../state/
ls -la $SRC/../state/treasury_guard.wasm
