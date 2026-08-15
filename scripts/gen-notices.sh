#!/usr/bin/env bash
# scripts/gen-notices.sh
#
# Generates THIRD-PARTY-NOTICES.md and stamps it with a hash of the inputs it was made from.
#
# `build.rs` recomputes that hash on release builds and refuses to build if it does not match, so
# a release binary cannot embed notices that have fallen behind the dependency graph. The file is
# gitignored: a committed copy would go stale the moment a dependency moved, and a stale notice is
# worse than none -- it names crates the binary no longer carries and omits ones it does.
#
# Installs cargo-about if it is missing, so this script is the only thing anyone needs to know
# about. Runs from anywhere: it moves to the repository root itself.
#
# Usage:
#   bash scripts/gen-notices.sh
set -euo pipefail

# Every path below, and cargo-about's own manifest lookup, is relative to the repository root.
# Going there rather than demanding the caller already be there removes the commonest way to run
# this wrongly, and makes the output land in one predictable place whatever the caller's cwd.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# `Cargo.lock` fixes every crate version, and crates.io versions are immutable, so these three
# files determine the output completely. Keep this list, and its order, identical to `build.rs`.
INPUTS=(Cargo.lock about.toml about.md.hbs)
OUTPUT=THIRD-PARTY-NOTICES.md

# All three are tracked files, so a missing one means it was deleted rather than that the caller
# is somewhere unexpected -- the `cd` above already settled that.
for f in "${INPUTS[@]}"; do
    if [ ! -f "$f" ]; then
        echo "gen-notices: $f is missing from $(pwd)." >&2
        echo "  It is a tracked file and the notices cannot be generated without it. Restore it:" >&2
        echo "      git checkout -- $f" >&2
        exit 1
    fi
done

# The binary is gated behind `cli`, a non-default feature: without it cargo installs the library
# and no executable at all, reporting only a warning. `--locked` builds it against its own
# lockfile rather than whatever resolves today.
if ! cargo about --version > /dev/null 2>&1; then
    echo "gen-notices: cargo-about not found, installing (this takes a minute)..." >&2
    cargo install --locked --features cli cargo-about
fi

cargo about generate about.md.hbs -o "$OUTPUT"

# Appended rather than templated in: the template renders what cargo-about knows about, and it
# knows nothing about the lockfile that decided its input.
hash=$(cat "${INPUTS[@]}" | sha256sum | cut -d' ' -f1)
printf '\n<!-- inputs-sha256: %s -->\n' "$hash" >> "$OUTPUT"

echo "gen-notices: wrote $OUTPUT ($(wc -c < "$OUTPUT") bytes, inputs-sha256 $hash)"
