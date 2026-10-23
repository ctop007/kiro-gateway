"""Tests for release_lib.py (version bump, GitHub Release manifest, tag notes, FORCE).

Run with:
    /opt/miniconda3/bin/python -I -m unittest discover -s desktop/scripts -p 'test_release_lib.py' -v
No network: the previous manifest is a local file.
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
BASE = "https://github.com/octo/kiro-gateway/releases/download/desktop-v0.0.2"


def make_fake_artifacts(version: str, bundle: Path) -> None:
    """Build outputs as Tauri names them (with spaces), placeholder content."""
    (bundle / "macos").mkdir(parents=True, exist_ok=True)
    (bundle / "dmg").mkdir(parents=True, exist_ok=True)
    (bundle / "macos/Kiro Gateway.app.tar.gz").write_bytes(b"placeholder package")
    (bundle / "macos/Kiro Gateway.app.tar.gz.sig").write_text("DRY-RUN-PLACEHOLDER-SIGNATURE\n", encoding="utf-8")
    (bundle / "dmg" / f"Kiro Gateway_{version}_universal.dmg").write_bytes(b"placeholder dmg")


def make_win_bundle(tmp: Path, version: str = V) -> Path:
    nsis = tmp / "winbundle" / "nsis"
    nsis.mkdir(parents=True)
    (nsis / f"Kiro Gateway_{version}_x64-setup.exe").write_bytes(b"w")
    (nsis / f"Kiro Gateway_{version}_x64-setup.exe.sig").write_text("WINSIG\n")
    return nsis.parent


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
    make_fake_artifacts(version, b)
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
            m = rl.build_manifest(V, "line1\nline2", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d)), BASE)
            self.assertEqual(rl.validate_manifest(m), [])
            self.assertEqual(m["version"], V)
            self.assertEqual(m["minRequiredVersion"], "0.0.1")
            self.assertEqual(set(m["platforms"]), {"darwin-aarch64", "darwin-x86_64"})
            p = m["platforms"]["darwin-aarch64"]
            self.assertEqual(p["url"], f"{BASE}/Kiro-Gateway.app.tar.gz")
            self.assertEqual(p["signature"], "DRY-RUN-PLACEHOLDER-SIGNATURE")
            self.assertEqual(m["manualDownload"]["darwin"], f"{BASE}/Kiro-Gateway_{V}_universal.dmg")
            urls = [e["url"] for e in m["platforms"].values()] + list(m["manualDownload"].values())
            self.assertTrue(all(" " not in u for u in urls), urls)

    def test_windows_entry_added_when_present(self):
        with tempfile.TemporaryDirectory() as d:
            m = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d), win=True), BASE)
            self.assertEqual(rl.validate_manifest(m), [])
            self.assertEqual(m["platforms"]["windows-x86_64"]["signature"], "WINSIG")
            self.assertTrue(m["manualDownload"]["windows"].endswith("x64-setup.exe"))

    def test_min_required_above_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(rl.ReleaseError):
                rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.3", self.staged(Path(d)), BASE)

    def test_empty_signature_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            staged = self.staged(Path(d))
            staged["mac_package_sig"].write_text("  \n")
            with self.assertRaises(rl.ReleaseError):
                rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged, BASE)

    def test_validate_manifest_reports_each_missing_field(self):
        with tempfile.TemporaryDirectory() as d:
            good = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", self.staged(Path(d)), BASE)
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


class ManifestBaseUrlTests(unittest.TestCase):
    def staged(self, tmp: Path):
        found = rl.collect_artifacts(V, make_bundle(tmp), None)
        return rl.stage_artifacts(V, found, tmp / "stage")

    def test_base_url_must_be_https_and_trailing_slash_is_tolerated(self):
        with tempfile.TemporaryDirectory() as d:
            staged = self.staged(Path(d))
            for bad in ["", "http://github.com/x", "github.com/x", "ftp://x"]:
                with self.assertRaises(rl.ReleaseError, msg=bad):
                    rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged, bad)
            m = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged, BASE + "///")
            self.assertEqual(m["manualDownload"]["darwin"], f"{BASE}/Kiro-Gateway_{V}_universal.dmg")

    def test_staged_names_are_space_free_and_are_exactly_what_the_manifest_points_at(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            found = rl.collect_artifacts(V, make_bundle(tmp), make_win_bundle(tmp))
            assets = tmp / "assets"
            staged = rl.stage_artifacts(V, found, assets)
            m = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged, BASE)
            names = {p.name for p in assets.iterdir()}
            self.assertTrue(all(" " not in n for n in names), names)
            self.assertEqual(rl.check_manifest_assets(m, assets, BASE), [])
            urls = {e["url"] for e in m["platforms"].values()} | set(m["manualDownload"].values())
            self.assertTrue({u.rsplit("/", 1)[1] for u in urls} <= names)

    def test_manifest_pointing_at_a_missing_or_foreign_file_is_reported(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            staged = self.staged(tmp)
            m = rl.build_manifest(V, "n", "2026-11-01T10:00:00+08:00", "0.0.1", staged, BASE)
            (tmp / "stage" / "Kiro-Gateway.app.tar.gz").unlink()
            self.assertTrue(rl.check_manifest_assets(m, tmp / "stage", BASE))
            self.assertTrue(rl.check_manifest_assets(m, tmp / "stage", "https://example.com/other"))
            m["manualDownload"]["darwin"] = BASE + "/has space.dmg"
            self.assertTrue(any("has space" in p for p in rl.check_manifest_assets(m, tmp / "stage", BASE)))


class TagNotesTests(unittest.TestCase):
    def test_plain_body_is_the_notes_and_not_forced(self):
        self.assertEqual(rl.split_tag_notes("修复 A\n新增 B\n"), ("修复 A\n新增 B", False))

    def test_marker_line_forces_and_is_removed(self):
        self.assertEqual(rl.split_tag_notes("修复 A\n[force-upgrade]\n新增 B"), ("修复 A\n新增 B", True))
        self.assertEqual(rl.split_tag_notes("[force-upgrade]\r\n修复 A\r\n"), ("修复 A", True))

    def test_marker_inside_a_sentence_or_with_spaces_does_not_force(self):
        for body in ["说明 [force-upgrade] 说明", " [force-upgrade]", "[force-upgrade] ", "[Force-Upgrade]"]:
            notes, force = rl.split_tag_notes(body + "\n别的内容")
            self.assertFalse(force, body)
            self.assertIn("别的内容", notes)

    def test_empty_notes_are_an_error_even_with_only_the_marker(self):
        for body in ["", "  \n\n", "[force-upgrade]\n", "[force-upgrade]\n\n  \n"]:
            with self.assertRaises(rl.ReleaseError, msg=repr(body)):
                rl.split_tag_notes(body)


class PreviousFloorTests(unittest.TestCase):
    def test_reads_the_floor_of_a_previous_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "prev.json"
            p.write_text(json.dumps({"minRequiredVersion": "0.0.3"}))
            self.assertEqual(rl.read_previous_min(p), "0.0.3")

    def test_missing_unreadable_or_invalid_floor_is_none(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertIsNone(rl.read_previous_min(None))
            self.assertIsNone(rl.read_previous_min(Path(d) / "absent.json"))
            for content in ["not json", "[]", "{}", '{"minRequiredVersion": 3}', '{"minRequiredVersion": "v1"}']:
                p = Path(d) / "prev.json"
                p.write_text(content)
                self.assertIsNone(rl.read_previous_min(p), content)


class PrepareCliTests(unittest.TestCase):
    """The CLI the workflow runs, end to end on a temp tree."""

    def run_prepare(self, tmp: Path, *extra: str):
        make_fake = make_bundle(tmp)
        args = ["prepare", "--version", V, "--base-url", BASE, "--bundle-dir", str(make_fake),
                "--windows-dir", str(make_win_bundle(tmp)), "--out", str(tmp / "out"), *extra]
        r = subprocess.run([sys.executable, "-I", str(DESKTOP / "scripts/release_lib.py"), *args],
                           capture_output=True, text=True)
        return r, tmp / "out"

    def test_writes_assets_manifest_and_notes_with_defaults_when_no_previous_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            body = tmp / "body.txt"
            body.write_text("修复 A\n")
            r, out = self.run_prepare(tmp, "--tag-body", str(body))
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("默认值 0.0.1", r.stdout)
            m = json.loads((out / "assets/latest.json").read_text(encoding="utf-8"))
            self.assertEqual(m["minRequiredVersion"], "0.0.1")
            self.assertEqual(m["notes"], "修复 A")
            self.assertEqual((out / "notes.md").read_text(encoding="utf-8"), "修复 A\n")
            self.assertEqual(sorted(p.name for p in (out / "assets").iterdir()), sorted([
                "Kiro-Gateway.app.tar.gz", "Kiro-Gateway.app.tar.gz.sig", f"Kiro-Gateway_{V}_universal.dmg",
                f"Kiro-Gateway_{V}_x64-setup.exe", f"Kiro-Gateway_{V}_x64-setup.exe.sig", "latest.json"]))
            self.assertEqual(rl.validate_manifest(m), [])

    def test_force_marker_sets_floor_to_this_version_and_stays_out_of_the_notes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            body = tmp / "body.txt"
            body.write_text("修复 A\n[force-upgrade]\n")
            r, out = self.run_prepare(tmp, "--tag-body", str(body))
            self.assertEqual(r.returncode, 0, r.stderr)
            m = json.loads((out / "assets/latest.json").read_text(encoding="utf-8"))
            self.assertEqual(m["minRequiredVersion"], V)
            self.assertNotIn("force-upgrade", m["notes"])

    def test_previous_floor_is_carried_over_and_a_broken_previous_file_falls_back(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            prev = tmp / "prev.json"
            prev.write_text(json.dumps({"minRequiredVersion": "0.0.2"}))
            r, out = self.run_prepare(tmp, "--notes", "说明", "--previous-manifest", str(prev))
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(json.loads((out / "assets/latest.json").read_text())["minRequiredVersion"], "0.0.2")
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            prev = tmp / "prev.json"
            prev.write_text("<html>")
            r, out = self.run_prepare(tmp, "--notes", "说明", "--previous-manifest", str(prev))
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(json.loads((out / "assets/latest.json").read_text())["minRequiredVersion"], "0.0.1")

    def test_empty_notes_bad_base_url_and_missing_notes_fail_without_a_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            body = tmp / "body.txt"
            body.write_text("[force-upgrade]\n")
            r, out = self.run_prepare(tmp, "--tag-body", str(body))
            self.assertEqual(r.returncode, 2)
            self.assertFalse((out / "assets/latest.json").exists())
        with tempfile.TemporaryDirectory() as d:
            r, out = self.run_prepare(Path(d))
            self.assertEqual(r.returncode, 2)
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            r = subprocess.run([sys.executable, "-I", str(DESKTOP / "scripts/release_lib.py"), "prepare", "--version", V,
                                "--base-url", "http://insecure.example", "--notes", "n",
                                "--bundle-dir", str(make_bundle(tmp)), "--out", str(tmp / "o")],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 2)
            self.assertFalse((tmp / "o/assets/latest.json").exists())

    def test_stale_build_outputs_of_another_version_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            r = subprocess.run([sys.executable, "-I", str(DESKTOP / "scripts/release_lib.py"), "prepare", "--version", "0.0.3",
                                "--base-url", BASE, "--notes", "n", "--bundle-dir", str(make_bundle(tmp, version="0.0.2")),
                                "--out", str(tmp / "o")], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2)


class NoCosLeftoversTests(unittest.TestCase):
    def test_release_lib_has_no_tencent_cos_code(self):
        text = (DESKTOP / "scripts/release_lib.py").read_text(encoding="utf-8")
        for needle in ("qcloud_cos", "myqcloud", "tccli", "COS_SECRET"):
            self.assertNotIn(needle, text)

    def test_removed_cos_scripts_stay_removed(self):
        for name in ("release.sh", "publish-from-github.sh"):
            self.assertFalse((DESKTOP / "scripts" / name).exists(), name)


class WorkflowTests(unittest.TestCase):
    """Guards the release job's wiring (text checks; the YAML is parsed separately in CI/verification)."""

    TEXT = (DESKTOP.parent / ".github/workflows/desktop-release.yml").read_text(encoding="utf-8")

    def test_manifest_urls_use_the_running_repository_not_a_hardcoded_one(self):
        self.assertIn("https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}", self.TEXT)
        self.assertNotIn("heyuan110", self.TEXT)

    def test_tag_object_is_fetched_with_full_history(self):
        self.assertIn("fetch-depth: 0", self.TEXT)
        self.assertIn("+refs/tags/${TAG}:refs/tags/${TAG}", self.TEXT)

    def test_only_staged_assets_are_uploaded_and_cos_is_gone(self):
        self.assertIn("files: release-out/assets/*", self.TEXT)
        self.assertNotIn("artifacts/**", self.TEXT)
        for needle in ("COS", "myqcloud", "qcloud"):
            self.assertNotIn(needle, self.TEXT)

    def test_artifacts_keep_their_per_platform_directories(self):
        self.assertNotIn("merge-multiple", self.TEXT)
        self.assertIn("artifacts/bundles-macOS/universal-apple-darwin/release/bundle", self.TEXT)
        self.assertIn("artifacts/bundles-Windows/release/bundle", self.TEXT)


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




class TestWorkflowKeepsTheForcedFloor(unittest.TestCase):
    """A failed fetch of the previous manifest must not lower minRequiredVersion."""

    def test_previous_manifest_failure_aborts_unless_first_release(self):
        text = (Path(__file__).resolve().parents[2] / ".github/workflows/desktop-release.yml").read_text(encoding="utf-8")
        step = text[text.index("- name: Previous latest.json"):text.index("- name: Build latest.json")]
        self.assertIn('if [ "$count" = "0" ]', step)
        self.assertIn("exit 1", step)
        self.assertNotIn("::warning::", step)
