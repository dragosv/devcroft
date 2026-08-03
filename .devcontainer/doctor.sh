#!/usr/bin/env bash
# Preflight for task 1.1. Everything here is a precondition the spike would
# otherwise fail on in a way that looks like a design problem.
set -uo pipefail

fail=0
rule() { printf '%s\n' "----------------------------------------------------"; }

echo "== host =="
printf 'kernel   : %s\n' "$(uname -sr)"
printf 'arch     : %s\n' "$(uname -m)"
printf 'nono     : %s\n' "$(nono --version 2>&1 | head -1)"
printf 'cargo    : %s\n' "$(cargo --version)"
rule

echo "== landlock =="
if landlock-abi; then
    :
else
    fail=1
fi
rule

# Lesson from task 1.2: on macOS, nono's baseline profile grants /private/tmp
# read+write, which silently made the "ungranted" probe path reachable and the
# confinement check vacuous. The Linux baseline is a different set, so the
# spike's "outside" path must be re-picked here rather than assumed.
echo "== baseline grants (pick an ungranted path for the spike) =="
for p in /tmp "$HOME" /var/tmp /workspaces; do
    [ -e "$p" ] || continue
    verdict=$(nono --silent why --path "$p/probe" --op read 2>&1 | head -1)
    printf '%-14s %s\n' "$p" "$verdict"
done
echo
echo "Use a DENIED path as the spike's scratch root; an ALLOWED one proves nothing."
rule

if [ "$fail" -ne 0 ]; then
    echo "NOT READY for task 1.1 — see the landlock failure above."
    exit 1
fi
echo "Ready for task 1.1."
