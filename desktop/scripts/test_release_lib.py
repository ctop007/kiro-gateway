"""Tests for release_lib.py (version bump, manifest, upload order, FORCE).

Run with:
    /opt/miniconda3/bin/python -I -m unittest discover -s desktop/scripts -p 'test_release_lib.py' -v
No network and no COS access: uploads and HEAD checks are injected fakes.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_lib as rl  # noqa: E402

DESKTOP = Path(__file__).resolve().parent.parent
V = "0.0.2"


def make_root(tmp: Path, version="0.0.1", lock=True) -> Path:
    (tmp / "src-tauri").mkdir(parents=True)
    (tmp / "package.json").write_text('{\n  "name": "x",\n  "version": "%s",\n  "dependencies": {"a": "1.0.0"}\n}\n' % version)
    (tmp / "src-tauri/tauri.conf.json").write_text('{\n  "productName": "K",\n  "version": "%s",\n  "x": {"version": "9.9.9"}\n}\n' % version)
    (tmp / "src-tauri/Cargo.toml").write_text(
        '[package]\nname = "k"\nversion = "%s"\nedition = "2021"\n\n[dependencies]\nserde = { version = "=1.0.228" }\n' % version)
    if lock:
        (tmp / "src-tauri/Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "aaa"\nversion = "1.0.0"\n\n[[package]]\nname = "kiro-gateway-desktop"\nversion = "%s"\n' % version)
    return tmp


def make_bundle(tmp: Path, version=V, sig=True, dmg=True) -> Path:
    b = tmp / "bundle"
    rl.make_fake_artifacts(version, b)
    if not sig:
        (b / "macos/Kiro Gateway.app.tar.gz.sig").unlink()
    if not dmg:
        for f in (b / "dmg").iterdir():
            f.unlink()
    return b


class VersionBumpTests(unittest.TestCase):
    def test_bump_rewrites_all_three_files_and_lock_only_at_the_right_places(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d))
            lines = rl.bump_versions(root, "0.0.2")
            self.assertEqual(len(lines), 4)
            self.assertEqual(rl.read_versions(root), {"package.json": "0.0.2", "tauri.conf.json": "0.0.2", "Cargo.toml": "0.0.2"})
            self.assertIn('"x": {"version": "9.9.9"}', (root / "src-tauri/tauri.conf.json").read_text())
            self.assertIn('serde = { version = "=1.0.228" }', (root / "src-tauri/Cargo.toml").read_text())
            lock = (root / "src-tauri/Cargo.lock").read_text()
            self.assertIn('name = "aaa"\nversion = "1.0.0"', lock)
            self.assertIn('name = "kiro-gateway-desktop"\nversion = "0.0.2"', lock)
            json.loads((root / "package.json").read_text())

    def test_dry_run_writes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d))
            rl.bump_versions(root, "0.0.2", write=False)
            self.assertEqual(set(rl.read_versions(root).values()), {"0.0.1"})

    def test_rejects_non_increasing_and_malformed_versions(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d))
            for bad in ["0.0.1", "0.0.0", "1.2", "v0.0.2", "0.0.2-beta", "", "01.0.0", "0.0.2 "]:
                with self.assertRaises(rl.ReleaseError, msg=bad):
                    rl.bump_versions(root, bad)
            self.assertEqual(set(rl.read_versions(root).values()), {"0.0.1"})

    def test_rejects_when_three_files_disagree_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d))
            (root / "package.json").write_text('{\n  "version": "0.0.9"\n}\n')
            with self.assertRaises(rl.ReleaseError):
                rl.bump_versions(root, "0.1.0")
            self.assertIn("0.0.1", (root / "src-tauri/Cargo.toml").read_text())

    def test_numeric_comparison_allows_0_0_10_after_0_0_9(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d), version="0.0.9")
            rl.bump_versions(root, "0.0.10")
            self.assertEqual(set(rl.read_versions(root).values()), {"0.0.10"})

    def test_works_without_cargo_lock(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d), lock=False)
            self.assertEqual(len(rl.bump_versions(root, "0.0.2")), 3)

    def test_matches_the_real_desktop_tree_in_dry_run(self):
        versions = rl.read_versions(DESKTOP)
        self.assertEqual(len(set(versions.values())), 1, versions)
        rl.bump_versions(DESKTOP, "99.0.0", write=False)


class ManifestTests(unittest.TestCase):
    def staged(self, tmp: Path, win=False):
        found = rl.collect_artifacts(V, make_bundle(tmp), None)
        staged = rl.stage_artifacts(V, found, tmp / "stage")
        if win:
            (tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe").write_bytes(b"w")
            (tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe.sig").write_text("WINSIG\n")
            staged.update(win_setup=tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe",
                          win_setup_sig=tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe.sig")
        return staged

    def test_manifest_has_every_field_and_both_mac_targets(self):
        with tempfile.TemporaryDirectory() as d:
            m = rl.build_manifest(V, "line1\nline2", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d)))
            self.assertEqual(rl.validate_manifest(m), [])
            self.assertEqual(m["version"], V)
            self.assertEqual(m["minRequiredVersion"], "0.0.1")
            self.assertEqual(set(m["platforms"]), {"darwin-aarch64", "darwin-x86_64"})
            p = m["platforms"]["darwin-aarch64"]
            self.assertEqual(p["url"], f"{rl.BASE_URL}/kiro-gateway/desktop/{V}/Kiro-Gateway.app.tar.gz")
            self.assertEqual(p["signature"], "DRY-RUN-PLACEHOLDER-SIGNATURE")
            self.assertEqual(m["manualDownload"]["darwin"], f"{rl.BASE_URL}/kiro-gateway/desktop/{V}/Kiro-Gateway_{V}_universal.dmg")
            urls = [e["url"] for e in m["platforms"].values()] + list(m["manualDownload"].values())
            self.assertTrue(all(" " not in u for u in urls), urls)

    def test_windows_entry_added_when_present(self):
        with tempfile.TemporaryDirectory() as d:
            m = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d), win=True))
            self.assertEqual(rl.validate_manifest(m), [])
            self.assertEqual(m["platforms"]["windows-x86_64"]["signature"], "WINSIG")
            self.assertTrue(m["manualDownload"]["windows"].endswith("x64-setup.exe"))

    def test_min_required_above_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(rl.ReleaseError):
                rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.3", self.staged(Path(d)))

    def test_empty_signature_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            staged = self.staged(Path(d))
            staged["mac_package_sig"].write_text("  \n")
            with self.assertRaises(rl.ReleaseError):
                rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged)

    def test_validate_manifest_reports_each_missing_field(self):
        with tempfile.TemporaryDirectory() as d:
            good = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d)))
        for field in ["version", "minRequiredVersion", "notes", "pub_date", "platforms", "manualDownload"]:
            broken = dict(good)
            del broken[field]
            self.assertTrue(rl.validate_manifest(broken), field)
        broken = json.loads(json.dumps(good))
        broken["platforms"]["darwin-aarch64"]["signature"] = ""
        self.assertTrue(rl.validate_manifest(broken))
        broken = json.loads(json.dumps(good))
        broken["platforms"]["linux-x86_64"] = broken["platforms"]["darwin-aarch64"]
        self.assertTrue(rl.validate_manifest(broken))
        self.assertTrue(rl.validate_manifest([]))

    def test_force_sets_floor_to_release_version(self):
        self.assertEqual(rl.resolve_min_required("0.0.3", True, "0.0.1"), "0.0.3")

    def test_non_force_keeps_previous_floor_so_forced_upgrade_is_not_lifted(self):
        self.assertEqual(rl.resolve_min_required("0.0.4", False, "0.0.3"), "0.0.3")
        self.assertEqual(rl.resolve_min_required("0.0.4", False, None), "0.0.1")
        self.assertEqual(rl.resolve_min_required("0.0.4", False, "garbage"), "0.0.1")
        self.assertEqual(rl.resolve_min_required("0.0.2", False, "0.0.9"), "0.0.2")

    def test_force_confirmation_needs_exact_version(self):
        self.assertTrue(rl.confirm_force("0.0.3", lambda: "0.0.3\n"))
        for typed in ["", "y", "yes", "0.0.2", "0.0.30", "v0.0.3"]:
            self.assertFalse(rl.confirm_force("0.0.3", lambda t=typed: t), typed)


class ArtifactTests(unittest.TestCase):
    def test_missing_signature_or_dmg_is_an_error(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(rl.ReleaseError):
                rl.collect_artifacts(V, make_bundle(Path(d), sig=False), None)
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(rl.ReleaseError):
                rl.collect_artifacts(V, make_bundle(Path(d), dmg=False), None)

    def test_dmg_of_another_version_is_not_picked(self):
        with tempfile.TemporaryDirectory() as d:
            b = make_bundle(Path(d), version="0.0.1")
            with self.assertRaises(rl.ReleaseError):
                rl.collect_artifacts(V, b, None)

    def test_no_bundle_directories_is_an_error(self):
        with self.assertRaises(rl.ReleaseError):
            rl.collect_artifacts(V, None, None)


class UploadOrderTests(unittest.TestCase):
    def plan(self, tmp: Path, win=False):
        found = rl.collect_artifacts(V, make_bundle(tmp), None)
        staged = rl.stage_artifacts(V, found, tmp / "stage")
        if win:
            for n, c in (("Kiro-Gateway_0.0.2_x64-setup.exe", b"w"), ("Kiro-Gateway_0.0.2_x64-setup.exe.sig", b"s")):
                (tmp / "stage" / n).write_bytes(c)
            staged.update(win_setup=tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe",
                          win_setup_sig=tmp / "stage/Kiro-Gateway_0.0.2_x64-setup.exe.sig")
        manifest = tmp / "latest.json"
        manifest.write_text("{}")
        return rl.build_plan(V, staged, manifest)

    def test_order_is_installer_then_package_then_sig_then_manifest_last(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d), win=True)
            self.assertEqual([s.kind for s in plan], ["installer", "installer", "package", "sig", "sig", "manifest"])
            self.assertEqual(plan[-1].key, "kiro-gateway/desktop/latest.json")
            self.assertTrue(all(s.key.startswith(f"kiro-gateway/desktop/{V}/") for s in plan[:-1]))
            self.assertIn("no-cache", plan[-1].cache_control)

    def test_run_upload_heads_every_file_before_the_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d))
            events = []
            rl.run_upload(plan, lambda s: events.append(("put", s.kind)),
                          lambda u: (events.append(("head", u.rsplit("/", 1)[-1])) or (200, None)), lambda m: None)
            kinds = [e[0] for e in events]
            last_head = max(i for i, k in enumerate(kinds) if k == "head")
            self.assertEqual(events[-1], ("put", "manifest"))
            self.assertLess(last_head, len(events) - 1)
            self.assertEqual(kinds.count("head"), len(plan) - 1)

    def test_head_failure_never_uploads_the_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d))
            put = []
            for status in (404, 0, 403):
                put.clear()
                with self.assertRaises(rl.ReleaseError):
                    rl.run_upload(plan, lambda s: put.append(s.kind), lambda u: (status, None), lambda m: None)
                self.assertNotIn("manifest", put)

    def test_head_size_mismatch_never_uploads_the_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d))
            put = []
            with self.assertRaises(rl.ReleaseError):
                rl.run_upload(plan, lambda s: put.append(s.kind), lambda u: (200, 1), lambda m: None)
            self.assertNotIn("manifest", put)

    def test_put_failure_stops_before_later_steps(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d))
            put = []

            def boom(step):
                put.append(step.kind)
                if len(put) == 2:
                    raise OSError("network")

            with self.assertRaises(OSError):
                rl.run_upload(plan, boom, lambda u: (200, None), lambda m: None)
            self.assertEqual(len(put), 2)

    def test_plan_without_manifest_last_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            plan = self.plan(Path(d))
            with self.assertRaises(rl.ReleaseError):
                rl.run_upload(list(reversed(plan)), lambda s: None, lambda u: (200, None), lambda m: None)


class CredentialTests(unittest.TestCase):
    def test_env_wins_and_file_is_fallback(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            (home / ".tccli").mkdir()
            (home / ".tccli/default.credential").write_text('{"secretId":"fid","secretKey":"fkey"}')
            self.assertEqual(rl.load_credentials({"COS_SECRET_ID": "eid", "COS_SECRET_KEY": "ekey"}, home), ("eid", "ekey"))
            self.assertEqual(rl.load_credentials({}, home), ("fid", "fkey"))
            self.assertEqual(rl.load_credentials({"COS_SECRET_ID": "eid"}, home), ("fid", "fkey"))

    def test_missing_or_broken_credentials_raise_without_leaking(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            with self.assertRaises(rl.ReleaseError):
                rl.load_credentials({}, home)
            (home / ".tccli").mkdir()
            (home / ".tccli/default.credential").write_text("not json SECRET-VALUE")
            with self.assertRaises(rl.ReleaseError) as cm:
                rl.load_credentials({}, home)
            self.assertNotIn("SECRET-VALUE", str(cm.exception))


class UpdaterKeyTests(unittest.TestCase):
    def test_pubkey_in_tauri_conf_is_a_minisign_public_key(self):
        import base64
        conf = json.loads((DESKTOP / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        pub = conf["plugins"]["updater"]["pubkey"]
        text = base64.b64decode(pub).decode("utf-8")
        self.assertTrue(text.startswith("untrusted comment: minisign public key"), text[:40])
        self.assertEqual(len(text.strip().splitlines()), 2)

    def test_pubkey_matches_local_key_when_present(self):
        pub_file = Path.home() / ".kiro-gateway/desktop-updater.key.pub"
        if not pub_file.is_file():
            self.skipTest("local signing key not generated on this machine")
        conf = json.loads((DESKTOP / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(conf["plugins"]["updater"]["pubkey"], pub_file.read_text(encoding="utf-8").strip())

    def test_no_private_key_material_in_the_desktop_tree(self):
        gi = (DESKTOP / ".gitignore").read_text(encoding="utf-8")
        self.assertIn("*.key", gi)
        for path in DESKTOP.glob("*.key"):
            self.fail(f"private key file inside desktop/: {path}")


class SigningEnvironmentTests(unittest.TestCase):
    """The Tauri 2 CLI reads TAURI_SIGNING_PRIVATE_KEY; the *_PATH spelling is ignored."""

    def test_release_script_exports_the_variable_tauri_reads(self):
        text = (DESKTOP / "scripts/release.sh").read_text()
        exports = [l.strip() for l in text.splitlines() if l.strip().startswith("export TAURI_SIGNING")]
        self.assertTrue(any(l.startswith("export TAURI_SIGNING_PRIVATE_KEY=") for l in exports), exports)
        self.assertFalse(any(l.startswith("export TAURI_SIGNING_PRIVATE_KEY_PATH=") for l in exports), exports)

    def test_missing_signature_hint_names_the_right_variable(self):
        text = (DESKTOP / "scripts/release_lib.py").read_text()
        self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY_PATH", text)


class DryRunScriptTests(unittest.TestCase):
    """release.sh --dry-run end to end: fake build, real manifest, no upload."""

    def run_script(self, *args, stdin=""):
        env = {"PATH": "/usr/bin:/bin:/usr/local/bin", "HOME": tempfile.mkdtemp(), "RELEASE_STAGING": self.staging}
        return subprocess.run(["bash", str(DESKTOP / "scripts/release.sh"), *args], input=stdin,
                              capture_output=True, text=True, env=env, timeout=60)

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.staging = self._tmp.name
        self.before = rl.read_versions(DESKTOP)

    def tearDown(self):
        self._tmp.cleanup()
        self.assertEqual(rl.read_versions(DESKTOP), self.before, "dry-run must not modify versions")

    def test_dry_run_prints_plan_and_writes_valid_manifest(self):
        r = self.run_script("VERSION=0.9.9", "NOTES=第一条\n第二条", "--dry-run")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("上传顺序", r.stdout)
        self.assertIn("kiro-gateway/desktop/latest.json", r.stdout)
        manifest = json.loads(Path(self.staging, "out/latest.json").read_text())
        self.assertEqual(rl.validate_manifest(manifest), [])
        self.assertEqual(manifest["version"], "0.9.9")
        self.assertEqual(manifest["minRequiredVersion"], "0.0.1")
        self.assertEqual(manifest["notes"], "第一条\n第二条")
        self.assertTrue(r.stdout.index("[installer]") < r.stdout.index("[package]") < r.stdout.index("[sig]") < r.stdout.index("[manifest]"))

    def test_force_requires_retyping_the_version(self):
        r = self.run_script("VERSION=0.9.9", "NOTES=n", "FORCE=1", "--dry-run", stdin="0.9.8\n")
        self.assertEqual(r.returncode, 1)
        self.assertFalse(Path(self.staging, "out/latest.json").exists())
        r = self.run_script("VERSION=0.9.9", "NOTES=n", "FORCE=1", "--dry-run", stdin="0.9.9\n")
        self.assertEqual(r.returncode, 0, r.stderr)
        manifest = json.loads(Path(self.staging, "out/latest.json").read_text())
        self.assertEqual(manifest["minRequiredVersion"], "0.9.9")

    def test_bad_arguments_fail_before_anything_happens(self):
        for args in (["NOTES=n", "--dry-run"], ["VERSION=0.9.9", "--dry-run"], ["VERSION=0.0.1", "NOTES=n", "--dry-run"],
                     ["VERSION=abc", "NOTES=n", "--dry-run"], ["VERSION=0.9.9", "NOTES=n", "FORCE=2", "--dry-run"], ["--bogus"]):
            r = self.run_script(*args)
            self.assertNotEqual(r.returncode, 0, args)
        self.assertFalse(Path(self.staging, "out").exists())

    def test_notes_can_come_from_a_file(self):
        notes = Path(self.staging, "notes.md")
        notes.write_text("- fix a\n- fix b\n", encoding="utf-8")
        r = self.run_script("VERSION=0.9.9", f"NOTES={notes}", "--dry-run")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(Path(self.staging, "out/latest.json").read_text())["notes"], "- fix a\n- fix b")

    def test_output_never_contains_credentials(self):
        r = self.run_script("VERSION=0.9.9", "NOTES=n", "--dry-run")
        text = r.stdout + r.stderr
        for secret in ("AKID", "secretKey", "secretId"):
            self.assertNotIn(secret, text)


class RollbackAndPublishedFloorTests(unittest.TestCase):
    def test_restore_puts_back_every_version_file_byte_for_byte(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d) / "desktop")
            originals = {rel: (root / rel).read_bytes() for rel in rl.VERSION_FILES}
            snap = Path(d) / "snap"
            self.assertEqual(sorted(rl.snapshot_version_files(root, snap)), sorted(rl.VERSION_FILES))
            rl.bump_versions(root, "0.0.2")
            self.assertNotEqual((root / "package.json").read_bytes(), originals["package.json"])
            rl.restore_version_files(root, snap)
            for rel, data in originals.items():
                self.assertEqual((root / rel).read_bytes(), data, rel)
            self.assertEqual(set(rl.read_versions(root).values()), {"0.0.1"})

    def test_restore_removes_a_half_written_temp_file(self):
        with tempfile.TemporaryDirectory() as d:
            root = make_root(Path(d) / "desktop", lock=False)
            snap = Path(d) / "snap"
            self.assertEqual(len(rl.snapshot_version_files(root, snap)), 3)
            (root / "package.json.release-tmp").write_text("half")
            rl.restore_version_files(root, snap)
            self.assertFalse((root / "package.json.release-tmp").exists())

    def test_non_force_release_aborts_when_the_live_manifest_cannot_be_read(self):
        with self.assertRaises(rl.ReleaseError) as ctx:
            rl.previous_min_for_release("http://x/latest.json", False, False, fetch=lambda url: None)
        self.assertIn("无法读取线上 latest.json", str(ctx.exception))
        self.assertIn("FIRST_RELEASE=1", str(ctx.exception))

    def test_first_release_and_force_skip_the_lookup(self):
        def boom(url):
            raise AssertionError("must not fetch")
        self.assertIsNone(rl.previous_min_for_release("u", False, True, fetch=boom))
        self.assertIsNone(rl.previous_min_for_release("u", True, False, fetch=boom))

    def test_readable_manifest_returns_its_floor(self):
        self.assertEqual(rl.previous_min_for_release("u", False, False, fetch=lambda url: "0.0.3"), "0.0.3")

    def test_unreachable_url_is_unreadable(self):
        self.assertIsNone(rl.fetch_previous_min("http://127.0.0.1:1/latest.json", timeout=1.0))


class RealReleaseScriptTests(unittest.TestCase):
    """release.sh without --dry-run against a throw-away copy of the tree, with a fake pnpm."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.desktop = make_root(base / "desktop")
        (self.desktop / "scripts").mkdir()
        for name in ("release.sh", "release_lib.py"):
            (self.desktop / "scripts" / name).write_bytes((DESKTOP / "scripts" / name).read_bytes())
        self.bin = base / "bin"
        self.bin.mkdir()
        self.key = base / "key"
        self.key.write_text("not a real key")
        self.home = base / "home"
        self.home.mkdir()
        self.originals = {rel: (self.desktop / rel).read_bytes() for rel in rl.VERSION_FILES}

    def tearDown(self):
        self._tmp.cleanup()

    def fake_pnpm(self, body):
        path = self.bin / "pnpm"
        path.write_text("#!/bin/bash\n" + body + "\n")
        path.chmod(0o755)

    def run_release(self, *args, manifest_url="http://127.0.0.1:1/latest.json", stdin=""):
        env = {
            "PATH": f"{self.bin}:/usr/bin:/bin:/usr/local/bin",
            "HOME": str(self.home),
            "TMPDIR": self._tmp.name,
            "TAURI_SIGNING_PRIVATE_KEY_PATH": str(self.key),
            "RELEASE_MANIFEST_URL": manifest_url,
        }
        return subprocess.run(["bash", str(self.desktop / "scripts/release.sh"), *args], capture_output=True, text=True,
                              input=stdin, env=env, timeout=120)

    def assert_versions_untouched(self):
        for rel, data in self.originals.items():
            self.assertEqual((self.desktop / rel).read_bytes(), data, rel)

    def test_build_failure_restores_the_version_files_and_a_rerun_is_accepted(self):
        self.fake_pnpm("echo build exploded >&2; exit 1")
        r = self.run_release("VERSION=0.0.2", "NOTES=n", "FIRST_RELEASE=1")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("恢复版本号文件", r.stderr)
        self.assert_versions_untouched()
        r = self.run_release("VERSION=0.0.2", "NOTES=n", "FIRST_RELEASE=1")
        self.assertNotIn("必须高于当前版本", r.stderr)
        self.assertIn("build exploded", r.stderr)
        self.assert_versions_untouched()

    def test_interrupt_during_the_build_restores_too(self):
        self.fake_pnpm("kill -TERM $PPID; sleep 5")
        r = self.run_release("VERSION=0.0.2", "NOTES=n", "FIRST_RELEASE=1")
        self.assertNotEqual(r.returncode, 0)
        self.assert_versions_untouched()

    def test_unreadable_live_manifest_aborts_before_anything_is_written(self):
        self.fake_pnpm("echo must not build >&2; exit 1")
        r = self.run_release("VERSION=0.0.2", "NOTES=n")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("无法读取线上 latest.json，为避免降低最低可用版本已中止；确认是首次发布请加 FIRST_RELEASE=1", r.stderr)
        self.assertNotIn("must not build", r.stderr)
        self.assert_versions_untouched()

    def test_unreadable_live_manifest_does_not_stop_a_forced_release(self):
        self.fake_pnpm("echo build reached >&2; exit 1")
        r = self.run_release("VERSION=0.0.2", "NOTES=n", "FORCE=1", stdin="0.0.2\n")
        self.assertNotIn("无法读取线上 latest.json", r.stderr)
        self.assertIn("build reached", r.stderr)
        self.assert_versions_untouched()

    def test_bad_first_release_value_is_rejected(self):
        r = self.run_release("VERSION=0.0.2", "NOTES=n", "FIRST_RELEASE=2")
        self.assertEqual(r.returncode, 2)
        self.assert_versions_untouched()


if __name__ == "__main__":
    unittest.main()
