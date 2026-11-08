#!/bin/sh
set -eu
export ZIG_GLOBAL_CACHE_DIR="$(dirname "$0")/../.zig-cache"
# cc-rs recognizes Clang and supplies Rust's four-part triple. Zig uses its
# three-part spelling; preserve every other argument exactly (including spaces).
for arg do
    shift
    case "$arg" in
        --target=aarch64-unknown-linux-musl) ;;
        *) set -- "$@" "$arg" ;;
    esac
done
# build_keeper_guest.py disables Rust's self-contained CRT so Zig supplies a
# consistent set of musl startup objects, libc and compiler runtime libraries.
exec zig cc -target aarch64-linux-musl "$@"
