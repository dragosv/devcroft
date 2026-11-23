#!/bin/sh
# Reproduces the 2026-11-22 measurement from the repository (macOS only).
#
#   spikes/macos-exec-redirect/run.sh [work-dir]
#
# Builds the interposition library and the two test programs, assembles a
# runtime directory from the repo's `test-row-sh` example (brush, with `sh`,
# `echo`, `pwd`, `sleep` links), writes a devbox project around them, brings
# it up, and runs both in-sandbox scripts through `devcroft exec`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
work=${1:-$(mktemp -d /private/tmp/dc-exec-redirect.XXXXXX)}
work=$(cd "$work" && pwd -P)
T="$work/itest"
mkdir -p "$T/rt"

(cd "$repo" && cargo build -q --release --example test-row-sh --features test-support)
cp "$repo/target/release/examples/test-row-sh" "$T/rt/test-row-sh"
for n in sh echo pwd sleep; do ln -sf test-row-sh "$T/rt/$n"; done
xcrun clang -dynamiclib -o "$T/libinterpose.dylib" "$here/interpose.c"
xcrun clang -o "$T/prog" "$here/prog.c"
xcrun clang -o "$T/prog2" "$here/prog2.c"
(cd "$here/goprobe" && go build -o "$T/goprobe" .)
cp "$here/inside.sh" "$here/inside2.sh" "$here/inside3.sh" "$here/s.sh" "$T/"
chmod +x "$T/s.sh"

cd "$work"
cat > devcroft.toml <<TOML
[sandbox]
name = "${DC_NAME:-dcexecredirect}"

[env]
provider = "devbox"
TOML
[ -f devbox.json ] || echo '{"packages": ["bash@latest", "gnumake@latest", "python3@latest", "nodejs@latest"]}' > devbox.json
[ -f devbox.lock ] || devbox install
dc=${DEVCROFT:-devcroft}
"$dc" up
echo "=== inside.sh (exported after the session shell started)"
"$dc" exec -- env T="$T" "$T/rt/sh" "$T/inside.sh" || true
echo "=== inside2.sh (a fresh shell started with the variable set)"
"$dc" exec -- env T="$T" "$T/rt/sh" "$T/inside2.sh" || true
echo "=== inside3.sh (group 0: Go, make, python, npm, node)"
"$dc" exec -- env T="$T" "$T/rt/sh" "$T/inside3.sh" || true
"$dc" down
