#!/bin/sh
# Run inside the dedicated Linux builder, with a copy of aarch64.config in $1.
set -eu
fragment=$(realpath "$1")
workdir=${2:-/tmp/devcroft-kernel}
version=6.12.95
source_sha256=a9e8c51fcb1e695d1d35dde5886cba579cb6f29c9646c5889f39d63841d4b9f6
mkdir -p "$workdir"
cd "$workdir"
if [ ! -f "linux-$version.tar.xz" ]; then
    curl --fail --location --retry 2 --max-time 300 \
        "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$version.tar.xz" \
        -o "linux-$version.tar.xz"
fi
printf '%s  %s\n' "$source_sha256" "linux-$version.tar.xz" | sha256sum -c -
if [ ! -d "linux-$version" ]; then
    tar -xJf "linux-$version.tar.xz"
fi
cd "linux-$version"
export KBUILD_BUILD_TIMESTAMP='2026-11-07 00:00:00 UTC'
export KBUILD_BUILD_USER=devcroft
export KBUILD_BUILD_HOST=kernel-builder
export KBUILD_BUILD_VERSION=1
make ARCH=arm64 KCONFIG_ALLCONFIG="$fragment" allnoconfig
for symbol in SECURITY_LANDLOCK VIRTIO_VSOCKETS VIRTIO_BLK VIRTIO_CONSOLE \
    VIRTIO_MMIO VIRTIO_BALLOON PAGE_REPORTING DEVTMPFS EXT4_FS PROC_FS TMPFS UNIX98_PTYS; do
    grep -qx "CONFIG_$symbol=y" .config || { echo "required kernel capability missing: $symbol" >&2; exit 1; }
done
grep -qx 'CONFIG_LSM="landlock"' .config
timeout 900 make ARCH=arm64 -j4 Image > ../kernel-build.log 2>&1
mkdir -p ../output
cp arch/arm64/boot/Image ../output/Image
cp .config ../output/config
make -s kernelrelease > ../output/KVER
sha256sum ../output/Image ../output/config > ../output/SHA256SUMS
{ uname -a; cc --version; ld --version; make --version; } > ../output/toolchain.txt
echo "Kernel build complete: $workdir/output"
