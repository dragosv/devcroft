#!/bin/sh
# cgroup v2 delegation for this devcontainer, so fleet's resource-control
# work (add-linux-agent-fleet D6) can be spiked and tested here.
#
# Runs as root through a scoped sudoers rule (Dockerfile). Two verbs:
#
#   setup       — postStartCommand, every container start. Makes the
#                 cgroup mount writable, moves every process into an
#                 `init` leaf (cgroup v2's no-internal-process rule: the
#                 namespace root cannot enable domain controllers while it
#                 holds processes), enables the available controllers, and
#                 hands `delegated/` to the dev user the way a delegating
#                 manager does — ownership of the directory plus
#                 cgroup.procs, cgroup.threads and cgroup.subtree_control.
#                 Controllers are made *available* to `delegated/` but not
#                 enabled inside it: enabling them top-down is D6's job, so
#                 it stays the delegatee's.
#   enter PID   — moves one of the caller's own processes into
#                 `delegated/session`. Needed because migration requires
#                 write access to the common ancestor's cgroup.procs, and a
#                 process starting in `init/` shares only the root with
#                 `delegated/`. Run it once per shell (`enter $$`); children
#                 then start inside the subtree and can be moved freely
#                 within it without root.
#
# This is the "manual delegation" D6 rejects *as fleet's runtime
# mechanism*. Here it is a test harness: it provides the property
# delegation is for (a subtree the user owns, with controllers available)
# without systemd, which this container does not run.
set -eu

CG=/sys/fs/cgroup
USER_NAME=vscode

die() { echo "cgroup-delegate: $*" >&2; exit 1; }

[ "$(stat -fc %T "$CG")" = cgroup2fs ] || die "$CG is not cgroup v2"

delegate_to() {
    chown "$USER_NAME:$USER_NAME" "$1" "$1/cgroup.procs" "$1/cgroup.threads" "$1/cgroup.subtree_control"
}

setup() {
    # Docker mounts the cgroup filesystem read-only for any container that
    # is not --privileged. Remounting needs CAP_SYS_ADMIN (runArgs).
    if findmnt -no OPTIONS "$CG" | tr , '\n' | grep -qx ro; then
        mount -o remount,rw "$CG" || die "remount rw failed (is --cap-add=SYS_ADMIN in runArgs?)"
    fi

    mkdir -p "$CG/init"
    # Processes can fork while this runs; loop until the root is empty. A
    # pid that exited between read and write is not an error. Not `[ -s ]`:
    # cgroupfs files report size 0 whatever they hold, so it is always false.
    i=0
    while [ -n "$(cat "$CG/cgroup.procs")" ]; do
        while read -r pid; do
            echo "$pid" > "$CG/init/cgroup.procs" 2>/dev/null || true
        done < "$CG/cgroup.procs"
        i=$((i + 1))
        [ "$i" -lt 50 ] || die "root cgroup would not drain: $(tr '\n' ' ' < "$CG/cgroup.procs")"
    done

    for c in $(cat "$CG/cgroup.controllers"); do
        # dash reports every failed builtin write as "I/O error", whatever
        # the errno, so say which controller failed.
        echo "+$c" > "$CG/cgroup.subtree_control" || die "could not enable $c (cgroup.type: $(cat "$CG/cgroup.type"))"
    done

    mkdir -p "$CG/delegated"
    delegate_to "$CG/delegated"
    echo "cgroup-delegate: $CG/delegated owned by $USER_NAME; available: $(cat "$CG/delegated/cgroup.controllers")"
}

enter() {
    pid=${1:-}
    case "$pid" in ''|*[!0-9]*) die "usage: enter PID" ;; esac
    [ -d "/proc/$pid" ] || die "no such process: $pid"
    # Only the invoking user's own processes: this runs as root, and the
    # sudoers rule lets the dev user call it with any argument.
    owner=$(awk '/^Uid:/ {print $2}' "/proc/$pid/status")
    [ "$owner" = "${SUDO_UID:-}" ] || die "process $pid is not owned by the invoking user"
    [ -d "$CG/delegated" ] || die "not set up; run: sudo cgroup-delegate setup"
    if [ ! -d "$CG/delegated/session" ]; then
        mkdir "$CG/delegated/session"
        delegate_to "$CG/delegated/session"
    fi
    echo "$pid" > "$CG/delegated/session/cgroup.procs"
}

case "${1:-}" in
    setup) setup ;;
    enter) shift; enter "$@" ;;
    *) die "usage: cgroup-delegate setup | enter PID" ;;
esac
