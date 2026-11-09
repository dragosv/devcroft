#!/bin/sh
# Re-lock the flox test row's checked-in environment
# (tests/fixtures/flox-row). Needs a FloxHub login (`flox auth login`):
# resolution is the one step flox will not do without one, which is exactly
# why the lock is checked in rather than produced at test time.
#
# The lock covers all four systems in the manifest's `options.systems`, so
# one run on any host serves every host the suite runs on.
set -eu
cd "$(dirname "$0")/../tests/fixtures/flox-row"
rm -f .flox/env/manifest.lock
# `flox list` locks a manifest that has no lock, without building or
# activating anything.
flox list
test -s .flox/env/manifest.lock
echo "locked: tests/fixtures/flox-row/.flox/env/manifest.lock"
