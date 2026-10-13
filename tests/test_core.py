import os
import shutil
import tempfile
import unittest

from appremover import cleaner, safety
from appremover.engine import Options, remove_program
from appremover.model import DIR, HIGH, MEDIUM, REGKEY, REGVALUE, SERVICE, Program, RegKey
from appremover.names import expand_vars, normalize, publisher_variants
from appremover.registry import UNINSTALL_PATH, list_programs
from appremover.scanner import Environment, scan_leftovers
from appremover.system import System
from appremover import uninstall


class FakeRegistry:
    """tree: {RegKey: {"values": {...}}} - subkeys are derived from the paths."""

    def __init__(self):
        self.keys = {}

    def add(self, hive, path, view=64, **values):
        parts = path.split("\\")
        for i in range(1, len(parts) + 1):
            self.keys.setdefault(RegKey(hive, "\\".join(parts[:i]), view), {})
        self.keys[RegKey(hive, path, view)].update(values)

    def exists(self, key):
        return key in self.keys

    def values(self, key):
        return dict(self.keys.get(key, {}))

    def subkeys(self, key):
        prefix = key.path + "\\"
        return sorted({
            k.path[len(prefix):] for k in self.keys
            if k.hive == key.hive and k.view == key.view
            and k.path.startswith(prefix) and "\\" not in k.path[len(prefix):]
        })


class RecordingSystem(System):
    def __init__(self, reg):
        self.reg = reg
        self.calls = []

    def kill_processes_in(self, d): self.calls.append(("kill_in", d))
    def kill_process_names(self, n): self.calls.append(("kill", tuple(n)))
    def delete_service(self, n): self.calls.append(("svc", n))
    def delete_task(self, n): self.calls.append(("task", n))
    def reg_export(self, key, file):
        self.calls.append(("export", key))
        with open(file, "w") as f:
            f.write("x")
    def reg_delete_key(self, key):
        self.calls.append(("delkey", key))
        for k in [k for k in self.reg.keys if k.hive == key.hive and k.view == key.view
                  and (k.path == key.path or k.path.startswith(key.path + "\\"))]:
            del self.reg.keys[k]
    def reg_delete_value(self, key, name):
        self.calls.append(("delval", key, name)); self.reg.keys[key].pop(name, None)


class NamesTests(unittest.TestCase):
    def test_normalize(self):
        self.assertEqual(normalize("7-Zip 23.01 (x64)"), "7zip")
        self.assertEqual(normalize("Adobe Creative Cloud"), "adobecreativecloud")

    def test_publisher_variants(self):
        v = publisher_variants("Adobe Systems Incorporated")
        self.assertIn("adobe", v)
        self.assertIn("adobesystems", v)

    def test_expand(self):
        self.assertEqual(expand_vars(r"%ProgramFiles(x86)%\A", {"programfiles(x86)": "C:/P"}), "C:/P\\A")


class UninstallTests(unittest.TestCase):
    def prog(self, **kw):
        return Program(name="X", key=RegKey("HKLM", UNINSTALL_PATH + "\\X"), **kw)

    def test_split(self):
        self.assertEqual(uninstall.split_command('"C:\\a b\\u.exe" /S'), ("C:\\a b\\u.exe", "/S"))
        self.assertEqual(uninstall.split_command("C:\\a b\\u.exe /S"), ("C:\\a b\\u.exe", "/S"))

    def test_msi_converted_to_silent_uninstall(self):
        g = "{12345678-1234-1234-1234-123456789ABC}"
        cmd, _ = uninstall.build_command(self.prog(uninstall_string=f"MsiExec.exe /I{g}"))
        self.assertEqual(cmd, f"msiexec.exe /x {g} /qn /norestart")

    def test_quiet_string_preferred_and_inno(self):
        p = self.prog(uninstall_string="u.exe", quiet_uninstall_string="u.exe /quiet")
        self.assertEqual(uninstall.build_command(p)[0], "u.exe /quiet")
        self.assertEqual(uninstall.build_command(p, silent=False)[0], "u.exe")
        inno = self.prog(uninstall_string='"C:\\A\\unins000.exe"')
        self.assertIn("/VERYSILENT", uninstall.build_command(inno)[0])

    def test_missing_uninstaller_detected(self):
        cmd, _ = uninstall.build_command(self.prog(uninstall_string='"C:\\gone\\u.exe" --x'))
        self.assertFalse(uninstall.uninstaller_available(cmd, lambda p: False))
        self.assertIsNone(uninstall.build_command(self.prog())[0])


class SafetyTests(unittest.TestCase):
    def test_protected(self):
        prot = ["/pf", "/pf/Common Files"]
        self.assertTrue(safety.is_protected_path("/pf", prot))
        self.assertTrue(safety.is_protected_path("/", prot))
        self.assertTrue(safety.is_protected_path("/pf/Common Files", prot))
        self.assertFalse(safety.is_protected_path("/pf/App", prot))

    def test_protected_keys(self):
        self.assertTrue(safety.is_protected_key(RegKey("HKLM", "Software")))
        self.assertTrue(safety.is_protected_key(RegKey("HKLM", "Software\\Microsoft")))
        self.assertFalse(safety.is_protected_key(RegKey("HKLM", "Software\\Adobe\\Adobe Creative Cloud")))


class FlowTests(unittest.TestCase):
    """Adobe Creative Cloud with its uninstaller missing."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        t = self.tmp
        self.pf86 = os.path.join(t, "Program Files (x86)")
        self.pd = os.path.join(t, "ProgramData")
        self.local = os.path.join(t, "Local")
        self.menu = os.path.join(t, "StartMenu")
        for d in (self.pf86, self.pd, self.local, self.menu):
            os.makedirs(d)
        self.loc = os.path.join(self.pf86, "Adobe", "Adobe Creative Cloud")
        os.makedirs(os.path.join(self.loc, "Utils"))
        open(os.path.join(self.loc, "x.dll"), "w").close()
        # unrelated Adobe product that must survive
        self.photoshop = os.path.join(self.pf86, "Adobe", "Adobe Photoshop 2024")
        os.makedirs(self.photoshop)
        os.makedirs(os.path.join(self.menu, "Adobe"))
        self.lnk = os.path.join(self.menu, "Adobe", "Adobe Creative Cloud.lnk")
        open(self.lnk, "w").close()
        open(os.path.join(self.menu, "Adobe", "Adobe Photoshop 2024.lnk"), "w").close()
        os.makedirs(os.path.join(self.pd, "Adobe", "Creative Cloud"))

        self.reg = FakeRegistry()
        self.ukey = RegKey("HKLM", UNINSTALL_PATH + "\\Adobe Creative Cloud", 32)
        self.reg.add("HKLM", UNINSTALL_PATH + "\\Adobe Creative Cloud", 32,
                     DisplayName="Adobe Creative Cloud", Publisher="Adobe Inc.",
                     DisplayVersion="6.0", InstallLocation=self.loc,
                     UninstallString=f'"{self.loc}\\Utils\\Creative Cloud Uninstaller.exe"')
        self.reg.add("HKLM", "Software\\Adobe\\Adobe Creative Cloud", 32, x=1)
        self.reg.add("HKLM", "Software\\Adobe\\Adobe Photoshop 2024", 32, x=1)
        self.reg.add("HKLM", "Software\\Microsoft\\Foo", 64)
        self.reg.add("HKCU", "Software\\Adobe\\Adobe Creative Cloud", 64, x=1)
        self.reg.add("HKLM", "SYSTEM\\CurrentControlSet\\Services\\AdobeUpdateService", 64, ImagePath="x")
        self.reg.add("HKLM", "SYSTEM\\CurrentControlSet\\Services\\Other", 64, ImagePath="C:\\Win\\o.exe")
        self.reg.add("HKCU", "Software\\Microsoft\\Windows\\CurrentVersion\\Run", 64,
                     CCStartup=f'"{self.loc}\\cc.exe" --bg', Keep="C:\\Other\\o.exe")
        self.env = Environment(
            registry=self.reg, file_roots=[self.pf86, self.pd, self.local],
            shortcut_roots=[self.menu], tasks_dir="", protected=[self.pf86, self.pd, self.local, self.menu, self.tmp],
            envvars={"ProgramFiles(x86)": self.pf86, "ProgramData": self.pd, "LOCALAPPDATA": self.local},
        )

    def program(self):
        (p,) = [p for p in list_programs(self.reg) if p.name == "Adobe Creative Cloud"]
        return p

    def test_list_programs(self):
        self.assertEqual(self.program().key, self.ukey)

    def test_scan_finds_only_this_app(self):
        found = {(i.kind, i.label()): i for i in scan_leftovers(self.program(), self.env)}
        labels = [l for _, l in found]
        self.assertIn(self.loc, labels)
        self.assertIn(self.lnk, labels)
        self.assertIn(os.path.join(self.pd, "Adobe", "Creative Cloud"), labels)
        self.assertIn((SERVICE, "AdobeUpdateService"), found)
        self.assertIn((REGVALUE, f"HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run [CCStartup]"), found)
        for bad in (self.photoshop, os.path.join(self.pf86, "Adobe")):
            self.assertNotIn(bad, labels)
        self.assertFalse(any("Other" in l or "Foo" in l for l in labels))
        self.assertFalse(any("Photoshop" in l for l in labels))
        self.assertFalse(any(l.endswith("\\Software\\Adobe") for l in labels))
        # nested items under the install dir are not listed separately
        self.assertFalse(any(l.startswith(self.loc + os.sep) for l in labels))

    def test_full_removal_without_uninstaller(self):
        sysm = RecordingSystem(self.reg)
        logs = []
        report = remove_program(self.program(), self.env, sysm, Options(), logs.append, backup_base=os.path.join(self.tmp, "bk"))
        self.assertTrue(any("Uninstaller unavailable" in m for m in logs))
        self.assertFalse(os.path.exists(self.loc))
        self.assertFalse(os.path.exists(self.lnk))
        self.assertFalse(os.path.exists(os.path.join(self.pf86, "Adobe", "Adobe Creative Cloud")))
        self.assertTrue(os.path.isdir(self.photoshop))  # sibling untouched
        self.assertTrue(os.path.isdir(os.path.join(self.pf86, "Adobe")))
        self.assertFalse(self.reg.exists(self.ukey))
        self.assertTrue(self.reg.exists(RegKey("HKLM", "Software\\Adobe\\Adobe Photoshop 2024", 32)))
        self.assertTrue(self.reg.exists(RegKey("HKLM", "Software\\Microsoft\\Foo", 64)))
        self.assertIn("Keep", self.reg.values(RegKey("HKCU", "Software\\Microsoft\\Windows\\CurrentVersion\\Run")))
        self.assertIn(("svc", "AdobeUpdateService"), sysm.calls)
        self.assertEqual([(r.item.label(), r.message) for r in report.results if not r.ok], [])
        self.assertTrue(os.listdir(os.path.join(self.tmp, "bk")))  # registry backups written

    def test_empty_publisher_folder_pruned(self):
        shutil.rmtree(self.photoshop)
        remove_program(self.program(), self.env, RecordingSystem(self.reg), Options(), lambda m: None, backup_base=os.path.join(self.tmp, "bk"))
        self.assertFalse(os.path.exists(os.path.join(self.pf86, "Adobe")))
        self.assertTrue(os.path.isdir(self.pf86))

    def test_dry_run_changes_nothing(self):
        sysm = RecordingSystem(self.reg)
        remove_program(self.program(), self.env, sysm, Options(dry_run=True), lambda m: None)
        self.assertEqual(sysm.calls, [])
        self.assertTrue(os.path.isdir(self.loc))
        self.assertTrue(self.reg.exists(self.ukey))

    def test_cleaner_refuses_protected(self):
        from appremover.model import Leftover
        res = cleaner.clean([Leftover(DIR, self.pf86), Leftover(REGKEY, "Software", key=RegKey("HKLM", "Software"))],
                            self.env, RecordingSystem(self.reg), self.tmp)
        self.assertFalse(any(r.ok for r in res))
        self.assertTrue(os.path.isdir(self.pf86))


if __name__ == "__main__":
    unittest.main()
