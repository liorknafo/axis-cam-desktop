#!/bin/bash
# Regenerates licenses/NOTICES.md from Cargo.lock. Needs `cargo install cargo-about --locked --features cli`.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo about generate --locked -c about.toml about.hbs -o licenses/NOTICES.md
