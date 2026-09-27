#!/bin/sh
# Build the MCP server and stage it as a Tauri sidecar, so that an installed
# app carries its own copy and the path recorded in Claude Desktop's settings
# keeps working. Tauri looks for the binary under its target triple.
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

cargo build --release -p midiss-mcp --manifest-path src-tauri/Cargo.toml
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p src-tauri/binaries
cp src-tauri/target/release/midiss-mcp "src-tauri/binaries/midiss-mcp-$triple"
echo "staged src-tauri/binaries/midiss-mcp-$triple"
