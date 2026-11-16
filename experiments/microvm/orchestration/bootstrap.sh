#!/bin/sh
# Usage: bootstrap.sh WORKDIR [--freeze-tools] SCRIPT [ARGS...]
#
# Host phase of the editor and workload image builds. The Lima system contributes only what runs here --
# busybox (sh, tar, sha256sum, awk, mount, unshare, pivot_root), sudo and its kernel -- and those are
# checked against host.expected before anything else runs. Every other tool, build.py's
# interpreter included, comes from the SHA-256 pinned orchestration root built below.
#
# Inputs staged by orchestration.py into WORKDIR: tools-apks/ with tools.sha256 (every APK),
# bootstrap.apk / keys.apk (file names of apk-tools-static and alpine-keys),
# tools.files (the closure to install), tools.installed (expected `apk info -v`),
# host.expected (the host contract). With --freeze-tools the contract is recorded,
# not enforced. SCRIPT (a file in WORKDIR) runs inside the root as /work/SCRIPT with ARGS.
set -eu
# Every tool is named through busybox: the VM also carries coreutils, util-linux and others
# from earlier provisioning, and PATH would otherwise resolve those instead of the contract's.
bb=/bin/busybox
T=/home/dragos.guest/devcroft-orchestration

if [ "${1:-}" = "--enter" ]; then  # root, inside the new mount namespace
    W=$2 uid=$3 gid=$4 script=$5
    shift 5
    $bb mount --bind "$T" "$T"
    $bb mount --bind "$W" "$T/work"
    $bb mount --bind /dev "$T/dev"
    $bb mount -t proc proc "$T/proc"
    cd "$T"
    $bb pivot_root . .oldroot
    cd /
    /bin/busybox umount -l /.oldroot  # the pinned root's own busybox from here on
    exec /usr/bin/env -i PATH=/usr/bin:/usr/sbin:/bin:/sbin HOME=/root BUILD_UID="$uid" BUILD_GID="$gid" \
        /usr/bin/python3 "/work/$script" "$@"
fi

W=$1
shift
cd "$W"

{ $bb uname -r
  $bb awk '/^P:/{p=substr($0,3)} /^V:/{v=substr($0,3); if (p=="busybox"||p=="sudo"||p=="musl"||p=="linux-virt") print p"-"v}' \
      /lib/apk/db/installed | $bb sort
} > host.actual
freeze=""
if [ "${1:-}" = "--freeze-tools" ]; then
    freeze=1
    shift
fi
script=$1
shift
if [ -n "$freeze" ]; then
    :
elif ! $bb cmp -s host.actual host.expected; then
    echo "orchestration host differs from its pinned contract:" >&2
    $bb diff host.expected host.actual >&2 || true
    exit 1
fi

(cd tools-apks && $bb sha256sum -c -s ../tools.sha256)
B=$($bb mktemp -d "$W/bootstrap.XXXXXX")
$bb tar -xzf "tools-apks/$($bb cat bootstrap.apk)" -C "$B" sbin/apk.static 2>/dev/null
$bb tar -xzf "tools-apks/$($bb cat keys.apk)" -C "$B" usr/share/apk/keys 2>/dev/null
[ -x "$B/sbin/apk.static" ] && [ -d "$B/usr/share/apk/keys/aarch64" ]

sudo $bb rm -rf "$T"  # This script owns only this explicitly named root.
$bb mkdir "$T"
# shellcheck disable=SC2046 # one APK file name per line, none containing spaces
sudo "$B/sbin/apk.static" --root "$T" --initdb --no-network --no-cache --force-non-repository \
    --keys-dir "$B/usr/share/apk/keys/aarch64" add $($bb sed "s|^|$W/tools-apks/|" tools.files) >/dev/null
sudo "$B/sbin/apk.static" --root "$T" info -v 2>/dev/null | $bb sort > tools.actual
$bb rm -rf "$B"
if ! $bb cmp -s tools.actual tools.installed; then
    echo "orchestration root differs from its pinned closure" >&2
    $bb diff tools.installed tools.actual >&2 || true
    exit 1
fi

sudo $bb mkdir -p "$T/work" "$T/dev" "$T/proc" "$T/.oldroot"
$bb printf '%s\n' https://dl-cdn.alpinelinux.org/alpine/v3.23/main https://dl-cdn.alpinelinux.org/alpine/v3.23/community \
    | sudo $bb tee "$T/etc/apk/repositories" >/dev/null
sudo $bb cp /etc/resolv.conf "$T/etc/resolv.conf"
# Enter with pivot_root in a private mount namespace, not chroot: apk 3 runs package scripts
# in a user namespace, which the kernel refuses to any chrooted process ("unshare: Invalid
# argument"). The namespace's mounts go away with it, so nothing needs unmounting here.
sudo $bb unshare --mount --propagation private $bb sh "$0" --enter "$W" "$($bb id -u)" "$($bb id -g)" "$script" "$@"
