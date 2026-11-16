"""macOS side of the pinned image orchestration shared by build_editor.py and build_workload.py.

The Lima VM's base image is digest-pinned and checked here. On the VM, bootstrap.sh checks the
few host components it executes against a pinned contract, builds the SHA-256 pinned
orchestration root and runs the image's build script inside it. Nothing is `apk add`ed to the VM.
"""
import hashlib
import json
from pathlib import Path
import re
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
LIMA_HOME = Path("/private/tmp/devcroft-kernel-builder")
PINS = ROOT / "fixtures/orchestration-packages-aarch64.json"
STORE = ROOT / "artifacts/apk-store"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def lima_image():
    """The instance's base image, as Lima's own config pins it (location and digest)."""
    text = (LIMA_HOME / "kernel-build/lima.yaml").read_text()
    location = re.search(r"^\s*- location:\s*(\S+)", text, re.M)
    image_digest = re.search(r"^\s*digest:\s*(\S+)", text, re.M)
    if not location or not image_digest:
        raise RuntimeError("Lima instance config has no digest-pinned image")
    return {"location": location.group(1), "digest": image_digest.group(1)}


def check_inputs(freeze_tools):
    """Before starting the VM: the orchestration pins exist and the VM's base image is the pinned one."""
    image = lima_image()
    if not freeze_tools:
        if not PINS.exists():
            raise RuntimeError("initial orchestration closure resolution requires --freeze-tools")
        if json.loads(PINS.read_text())["lima_image"] != image:
            raise RuntimeError(("Lima base image differs from the pinned one", image))
    return image


def stage_store(lima, remote, out, subdirs, names=None):
    """Copy the host APK store (or only the named files) into each of remote/<subdir>.

    Alpine branch mirrors keep only the newest revision of each package, so a hash pin to a
    dl-cdn URL stops resolving once the package is updated. Every APK a build used is kept in
    this store and staged back in; each is still checked against its pinned SHA-256 on use.
    """
    STORE.mkdir(parents=True, exist_ok=True)
    stored = sorted(f for f in STORE.glob("*.apk") if names is None or f.name in names)
    if not stored:
        return
    bundle = out / "apk-store.tar"
    with tarfile.open(bundle, "w") as tar:
        for apk in stored:
            tar.add(apk, arcname=apk.name)
    lima("copy", bundle, f"kernel-build:{remote}/apk-store.tar")
    bundle.unlink()
    dirs = " ".join(f"{remote}/{sub}" for sub in subdirs)
    lima("shell", "kernel-build", "/bin/busybox", "sh", "-c",
         f"for d in {dirs}; do /bin/busybox mkdir -p $d && /bin/busybox tar -xf {remote}/apk-store.tar -C $d; done")


def keep_in_store(tar, members):
    """members: (path inside tar, pin record). Copy each into the store if absent; verify its hash."""
    for path, item in members:
        target = STORE / item["file"]
        if not target.exists():
            target.write_bytes(tar.extractfile(path).read())
        if digest(target) != item["sha256"]:
            raise RuntimeError(f"stored APK mismatch: {item['file']}")


def run(lima, remote, out, script, args, freeze_tools):
    """Stage the bootstrap and its lists into remote, then run script inside the pinned root.

    Returns (tools manifest, host contract lines). With freeze_tools the closure is first
    re-resolved on the VM (resolution only) and the contract is recorded, not enforced.
    """
    lima("copy", HERE / "bootstrap.sh", f"kernel-build:{remote}/bootstrap.sh")
    if freeze_tools:
        lima("copy", HERE / "freeze.py", f"kernel-build:{remote}/freeze.py")
        lima("shell", "kernel-build", "python3", f"{remote}/freeze.py", remote)
        lima("copy", f"kernel-build:{remote}/tools-packages.json", out / "tools-packages.json")
        tools = json.loads((out / "tools-packages.json").read_text())
    else:
        tools = json.loads(PINS.read_text())
    manifest = {k: tools[k] for k in ("schema", "bootstrap", "requested", "packages")}
    everything = [*tools["bootstrap"], *tools["packages"]]
    lists = {
        "tools.sha256": "".join(f"{x['sha256']}  {x['file']}\n" for x in everything),
        "bootstrap.apk": tools["bootstrap"][0]["file"] + "\n",
        "keys.apk": tools["bootstrap"][1]["file"] + "\n",
        "tools.files": "".join(f"{x['file']}\n" for x in tools["packages"]),
        "tools.installed": "".join(f"{n}\n" for n in sorted(x["file"].removesuffix(".apk") for x in tools["packages"])),
        "tools-packages.json": json.dumps(manifest, indent=2) + "\n",
    }
    if not freeze_tools:
        lists["host.expected"] = "".join(f"{line}\n" for line in tools["host"])
    for name, text in lists.items():
        (out / name).write_text(text)
        lima("copy", out / name, f"kernel-build:{remote}/{name}")
    lima("shell", "kernel-build", "/bin/busybox", "sh", f"{remote}/bootstrap.sh", remote,
         *(["--freeze-tools"] if freeze_tools else []), script, *args)
    lima("copy", f"kernel-build:{remote}/host.actual", out / "host.actual")
    return manifest, (out / "host.actual").read_text().split()


def keep_tools(tar, manifest, subdir="tools-apks"):
    keep_in_store(tar, [(f"{subdir}/{x['file']}", x) for x in [*manifest["bootstrap"], *manifest["packages"]]])


def write_pins(manifest, host, image):
    PINS.write_text(json.dumps(dict(manifest, host=host, lima_image=image), indent=2) + "\n")
