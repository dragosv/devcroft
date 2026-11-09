"""Exercise the compiled helper's fail-closed launch interface, without a VM."""
import copy
import fcntl
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
HELPER = ROOT / "libkrun/target/debug/devcroft-krun-spike"


class LaunchValidation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not HELPER.is_file():
            raise RuntimeError("build the libkrun helper before running these tests")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.instance = self.root / "instance"
        self.instance.mkdir(mode=0o700)
        self.cfg = {"schema": 1, "vcpus": 1, "memory_mib": 256,
                    "instance_dir": str(self.instance), "state": str(self.instance / "state.img")}
        Path(self.cfg["state"]).write_bytes(b"\0" * 8192)
        for key in ("kernel", "initramfs", "runtime"):
            file = self.root / key
            file.write_bytes((key + "-fixture").encode())
            self.cfg[key] = {"path": str(file), "sha256": hashlib.sha256(file.read_bytes()).hexdigest()}

    def check(self, cfg=None, raw=None):
        launch = self.root / "launch.json"
        launch.write_bytes(raw if raw is not None else json.dumps(cfg or self.cfg).encode())
        return subprocess.run([str(HELPER), "check", str(launch)], capture_output=True, text=True,
                              env={"PATH": "/usr/bin:/bin"}, timeout=5)

    def test_valid_inputs_are_not_runtime_qualification(self):
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        info = json.loads(result.stdout)
        self.assertFalse(info["nic"])
        self.assertFalse(info["tsi"])
        self.assertEqual(info["qualification"], "not-run")

    def test_provenance_identifies_the_reviewed_patch(self):
        result = subprocess.run([str(HELPER), "provenance"], capture_output=True,
                                text=True, check=True, timeout=5)
        info = json.loads(result.stdout)
        self.assertEqual(info["revision"], "6151710f4ea627cf0e5dbce78e4192fb4d64c2e9")
        patch = ROOT / "patches/libkrun-vsock-host-half-close.patch"
        self.assertEqual(info["patch_sha256"], hashlib.sha256(patch.read_bytes()).hexdigest())

    def test_corrupted_input_is_refused(self):
        Path(self.cfg["kernel"]["path"]).write_bytes(b"different-content")
        result = self.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)

    def test_unknown_network_switch_is_refused(self):
        cfg = copy.deepcopy(self.cfg)
        cfg["network"] = True
        self.assertIn("unknown field", self.check(cfg).stderr)

    def test_state_cannot_alias_an_input(self):
        cfg = copy.deepcopy(self.cfg)
        cfg["kernel"] = {"path": cfg["state"],
                         "sha256": hashlib.sha256(Path(cfg["state"]).read_bytes()).hexdigest()}
        self.assertIn("aliases an immutable input", self.check(cfg).stderr)

    def test_shared_instance_directory_is_refused(self):
        self.instance.chmod(0o755)
        self.assertIn("mode 0700", self.check().stderr)

    def test_immutable_image_inside_writable_scope_is_refused(self):
        cfg = copy.deepcopy(self.cfg)
        file = self.instance / "runtime.img"
        file.write_bytes(b"immutable-fixture")
        cfg["runtime"] = {"path": str(file), "sha256": hashlib.sha256(file.read_bytes()).hexdigest()}
        self.assertIn("outside the writable instance", self.check(cfg).stderr)

    def test_resource_budgets_are_bounded(self):
        for key, value in (("vcpus", 0), ("vcpus", 9), ("memory_mib", 0), ("memory_mib", 4097)):
            with self.subTest(key=key, value=value):
                cfg = copy.deepcopy(self.cfg)
                cfg[key] = value
                self.assertIn("1..8 vCPUs", self.check(cfg).stderr)

    def test_oversized_record_is_refused(self):
        self.assertIn("exceeds 64 KiB", self.check(raw=b" " * 65537).stderr)

    def test_stale_endpoints_and_dangling_links_are_refused(self):
        for name in ("control.sock", "keeper.sock", "ssh.sock"):
            with self.subTest(endpoint=name):
                path = self.instance / name
                path.symlink_to(self.instance / "missing-target")
                try:
                    self.assertIn("socket is occupied", self.check().stderr)
                finally:
                    path.unlink()

    def workload_config(self):
        cfg = copy.deepcopy(self.cfg)
        image = self.root / "toolchain.img"
        image.write_bytes(b"immutable-tools" + b"\0" * 8192)
        cfg["workload"] = {"path": str(image), "sha256": hashlib.sha256(image.read_bytes()).hexdigest()}
        cfg["data"] = str(self.instance / "data.img")
        Path(cfg["data"]).write_bytes(b"\0" * 8192)
        return cfg

    def test_optional_disks_must_be_paired(self):
        cfg = self.workload_config()
        for missing in ("workload", "data"):
            with self.subTest(missing=missing):
                partial = copy.deepcopy(cfg)
                del partial[missing]
                self.assertIn("provided together", self.check(partial).stderr)

    def test_toolchain_content_is_hash_checked(self):
        cfg = self.workload_config()
        self.assertEqual(self.check(cfg).returncode, 0)
        Path(cfg["workload"]["path"]).write_bytes(b"changed")
        self.assertIn("SHA-256 mismatch", self.check(cfg).stderr)

    def test_data_cannot_alias_bootstrap_state(self):
        cfg = self.workload_config()
        cfg["data"] = cfg["state"]
        self.assertIn("aliases state", self.check(cfg).stderr)

    def test_data_cannot_escape_instance_directory(self):
        cfg = self.workload_config()
        disk = self.root / "outside-data.img"
        disk.write_bytes(b"\0" * 8192)
        cfg["data"] = str(disk)
        self.assertIn("directly inside", self.check(cfg).stderr)

    def test_data_already_in_use_is_refused_before_confinement_or_boot(self):
        cfg = self.workload_config()
        launch = self.root / "launch.json"
        launch.write_text(json.dumps(cfg))
        with Path(cfg["data"]).open("r+b") as disk:
            fcntl.flock(disk, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run([str(HELPER), "run", str(launch)], capture_output=True,
                                    text=True, env={"PATH": "/usr/bin:/bin"}, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("private disk is already in use", result.stderr)


if __name__ == "__main__":
    unittest.main()
