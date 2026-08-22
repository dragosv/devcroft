# Linux/Landlock environment for task 1.1 — the half of the phase-1 spike
# that cannot run on a macOS host.
#
# Matches the workspace rust-version of both devcroft and nono (1.95); cargo
# is here to build devcroft itself, not nono.
FROM rust:1.95-bookworm

# The official image installs the minimal profile (rustc/cargo only, no
# clippy/rustfmt). devcontainer.json runs rust-analyzer with
# check.command = "clippy", so without this it fails on every file save.
RUN rustup component add clippy rustfmt

# nono's Homebrew formula marks pkgconf and rust as `=> :build`, but the
# Linux dbus dependency carries no such marker — it is needed at runtime, so
# the prebuilt binary needs the library even though we never compile nono.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/*

# Official installer (https://nono.sh/install.sh): fetches the prebuilt
# release for this arch and verifies it against the SHA256SUMS.txt published
# with the release. NONO_VERSION pins the tag — without it the script follows
# the 'latest' redirect, which would make the image drift between rebuilds and
# break the one thing task 1.3 needs: a known, tested nono version.
#
# Pinned to the same version used for the Seatbelt spike (task 1.2), so the
# two platform results are comparable.
#
# No longer a runtime dependency of the process tier itself (`use-nono-library`:
# the keeper links the `nono` crate directly and self-restricts, no `nono
# wrap` exec left anywhere). Kept installed anyway — this devcontainer is a
# development environment, not an end-user install, and the binary stays
# useful for manual `nono profile`/`nono why`/`nono wrap` comparisons against
# the library's own behavior (exactly how `own-policy-baseline` and
# `use-nono-library` were both actually investigated) and for
# `add-gvisor-backend`-adjacent debugging.
ARG NONO_VERSION=v0.71.0
RUN NONO_VERSION="${NONO_VERSION}" NONO_INSTALL_DIR=/usr/local/bin \
        sh -c 'curl -fsSL https://nono.sh/install.sh | sh' \
    && nono --version

COPY landlock-abi.c /tmp/landlock-abi.c
RUN cc -O2 -o /usr/local/bin/landlock-abi /tmp/landlock-abi.c && rm /tmp/landlock-abi.c

COPY doctor.sh /usr/local/bin/spike-doctor
RUN chmod +x /usr/local/bin/spike-doctor

ENV NONO_NO_UPDATE_CHECK=1

# flox provider (task 3.2). Installed via the official .deb, architecture-
# detected, and verified against the published SHA256SUMS — same posture as
# nono's pinned, checksummed install above. FLOX_VERSION pins the release
# for the same reason NONO_VERSION does: a drifting image cannot support a
# task whose whole point is behavior verified against a known version.
#
# UNVERIFIED beyond this build step: this Dockerfile change has not been
# exercised through an actual container rebuild. flox additionally needs a
# running nix-daemon, which normally starts via systemd; this image has no
# init system, so the sudoers rule and postStartCommand below start it
# explicitly instead. If `flox --version` fails here, the build itself
# will fail loudly — but daemon startup at container boot can only be
# confirmed by actually rebuilding.
ARG FLOX_VERSION=1.14.0
RUN set -eu; \
    cd /tmp; \
    case "$(dpkg --print-architecture)" in \
        amd64) FLOX_ARCH=x86_64 ;; \
        arm64) FLOX_ARCH=aarch64 ;; \
        *) echo "unsupported architecture: $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    DEB="flox-${FLOX_VERSION}.${FLOX_ARCH}-linux.deb"; \
    curl -fsSLO "https://downloads.flox.dev/by-env/stable/deb/${DEB}"; \
    curl -fsSL "https://downloads.flox.dev/by-env/stable/flox-${FLOX_VERSION}.SHA256SUMS" \
        | grep "deb/${DEB}\$" | awk -v f="$DEB" '{print $1"  "f}' | sha256sum -c -; \
    apt-get update && apt-get install -y --no-install-recommends "./${DEB}" \
    && rm -rf /var/lib/apt/lists/* "/tmp/${DEB}" \
    && flox --version

# gVisor backend (add-gvisor-backend). Installed via the official release
# bucket, architecture-detected, and verified against the published
# per-arch SHA512 — same posture as nono's and flox's pinned, checksummed
# installs above. RUNSC_VERSION pins the release for the same reason
# NONO_VERSION/FLOX_VERSION do: gVisor ships continuously as dated
# releases, not semver (add-gvisor-backend's own Open Questions section
# notes this explicitly, and is the reason `doctor`'s gvisor-backend check
# reports the version found rather than claiming a "tested range" this
# project hasn't decided on) — an unpinned `latest` would make the image
# drift between rebuilds.
#
# UNVERIFIED beyond this build step, same posture as flox's install above:
# this has not been exercised through an actual container rebuild (no
# docker socket is reachable from inside a running devcontainer to test
# one). Installing the binary here is necessary but not sufficient for the
# hardened tier to actually work — see devcontainer.json's own comment for
# the other half (rootless gVisor's unprivileged-userns requirement, which
# this container cannot currently satisfy).
ARG RUNSC_VERSION=20260810.0
RUN set -eu; \
    cd /tmp; \
    case "$(dpkg --print-architecture)" in \
        amd64) RUNSC_ARCH=x86_64 ;; \
        arm64) RUNSC_ARCH=aarch64 ;; \
        *) echo "unsupported architecture: $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    BASE="https://storage.googleapis.com/gvisor/releases/release/${RUNSC_VERSION}/${RUNSC_ARCH}"; \
    curl -fsSLO "${BASE}/runsc"; \
    curl -fsSL "${BASE}/runsc.sha512" | sha512sum -c -; \
    chmod +x runsc \
    && mv runsc /usr/local/bin/runsc \
    && runsc --version

# Drop root. Not only lint hygiene: devcroft runs as an unprivileged user, so
# a confinement spike executed as root would be measuring a process that does
# not resemble the one being designed for. The official rust image already
# chmods CARGO_HOME and RUSTUP_HOME a+w, so the toolchain stays usable here.
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID
RUN groupadd --gid "$USER_GID" "$USERNAME" \
    && useradd --uid "$USER_UID" --gid "$USER_GID" --create-home --shell /bin/bash "$USERNAME" \
    # Pre-create the Claude config dir owned by the user. A named volume
    # mounted over an existing path inherits that path's contents and
    # ownership on first use, so this avoids needing sudo + chown at runtime
    # just to make the mount writable.
    && mkdir -p "/home/$USERNAME/.claude" \
    && chown -R "$USER_UID:$USER_GID" "/home/$USERNAME"

# Scoped NOPASSWD rule for the one root action a session actually needs —
# starting flox's nix-daemon — rather than a blanket sudo grant. Paired
# with devcontainer.json's postStartCommand, which runs this on every
# container start (not just creation), since the daemon does not persist
# across restarts without an init system to supervise it.
RUN apt-get update && apt-get install -y --no-install-recommends sudo \
    && rm -rf /var/lib/apt/lists/* \
    && echo "$USERNAME ALL=(root) NOPASSWD: /usr/sbin/service nix-daemon *" \
        > /etc/sudoers.d/nix-daemon \
    && chmod 0440 /etc/sudoers.d/nix-daemon

USER $USERNAME
