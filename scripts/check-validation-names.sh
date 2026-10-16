#!/usr/bin/env bash
# Checks that every backticked snake_case name in docs/VALIDATION.md that
# reads as a test name (a path such as `control::tests::name`, or a name of
# four words or more) exists as `fn <name>` in src/, so that a renamed or
# deleted test cannot stay cited. Tests of the uc100 driver crate are looked
# up in its checkout under ~/.cargo/git/checkouts when there is one; a module
# such as `model::path_json` counts as `mod <name>`.
# Run it from anywhere: ./scripts/check-validation-names.sh
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
doc="$root/docs/VALIDATION.md"
dirs=("$root/src")
for d in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/uc100-driver-*/*; do
    [ -d "$d" ] && dirs+=("$d")
done
missing=0
while IFS= read -r name; do
    if ! grep -rqE --include='*.rs' "(fn|mod) ${name}\b" "${dirs[@]}"; then
        echo "docs/VALIDATION.md cites \`$name\`, but no \`fn $name\` is in src/ or the driver crate" >&2
        missing=1
    fi
done < <(
    grep -oE '`([a-z][a-z0-9_]*::)*[a-z][a-z0-9]*(_[a-z0-9]+)+`' "$doc" \
        | tr -d '`' \
        | awk -F'::' '{ n = split($NF, w, "_"); if (NF > 1 || n >= 4) print $NF }' \
        | sort -u
)
exit "$missing"
