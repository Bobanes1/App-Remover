"""Registry access (behind a small interface so it can be faked in tests) and
enumeration of installed programs."""
from __future__ import annotations

import sys
from typing import Any, Dict, List, Protocol

from .model import Program, RegKey

UNINSTALL_PATH = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"

UNINSTALL_ROOTS = [
    RegKey("HKLM", UNINSTALL_PATH, 64),
    RegKey("HKLM", UNINSTALL_PATH, 32),
    RegKey("HKCU", UNINSTALL_PATH, 64),
]


class RegistryBackend(Protocol):
    def subkeys(self, key: RegKey) -> List[str]: ...
    def values(self, key: RegKey) -> Dict[str, Any]: ...
    def exists(self, key: RegKey) -> bool: ...


class WinRegistry:
    """Real registry via ``winreg`` (Windows only)."""

    def __init__(self) -> None:
        import winreg  # noqa: F401  (raises ImportError off-Windows)

        self._w = winreg

    def _open(self, key: RegKey):
        w = self._w
        hive = w.HKEY_LOCAL_MACHINE if key.hive == "HKLM" else w.HKEY_CURRENT_USER
        flag = w.KEY_WOW64_64KEY if key.view == 64 else w.KEY_WOW64_32KEY
        return w.OpenKey(hive, key.path, 0, w.KEY_READ | flag)

    def exists(self, key: RegKey) -> bool:
        try:
            with self._open(key):
                return True
        except OSError:
            return False

    def subkeys(self, key: RegKey) -> List[str]:
        out: List[str] = []
        try:
            with self._open(key) as h:
                i = 0
                while True:
                    try:
                        out.append(self._w.EnumKey(h, i))
                    except OSError:
                        break
                    i += 1
        except OSError:
            pass
        return out

    def values(self, key: RegKey) -> Dict[str, Any]:
        out: Dict[str, Any] = {}
        try:
            with self._open(key) as h:
                i = 0
                while True:
                    try:
                        name, data, _ = self._w.EnumValue(h, i)
                    except OSError:
                        break
                    out[name] = data
                    i += 1
        except OSError:
            pass
        return out


def default_backend() -> RegistryBackend:
    if sys.platform != "win32":
        raise RuntimeError("App Remover only works on Windows.")
    return WinRegistry()


_SKIP_RELEASE = {"update", "hotfix", "security update", "update rollup", "service pack"}


def _str(v: Any) -> str:
    return v.strip() if isinstance(v, str) else ""


def list_programs(reg: RegistryBackend) -> List[Program]:
    programs: List[Program] = []
    seen = set()
    for root in UNINSTALL_ROOTS:
        for sub in reg.subkeys(root):
            key = root.child(sub)
            v = reg.values(key)
            name = _str(v.get("DisplayName"))
            if not name:
                continue
            if v.get("SystemComponent") == 1:
                continue
            if v.get("ParentKeyName") or _str(v.get("ReleaseType")).lower() in _SKIP_RELEASE:
                continue
            if name.startswith("KB") and name[2:8].isdigit():
                continue
            size = v.get("EstimatedSize")
            prog = Program(
                name=name,
                key=key,
                publisher=_str(v.get("Publisher")),
                version=_str(v.get("DisplayVersion")),
                install_location=_str(v.get("InstallLocation")).strip('"'),
                uninstall_string=_str(v.get("UninstallString")),
                quiet_uninstall_string=_str(v.get("QuietUninstallString")),
                size_kb=size if isinstance(size, int) else 0,
                install_date=_str(v.get("InstallDate")),
                windows_installer=v.get("WindowsInstaller") == 1,
            )
            ident = (name.lower(), prog.version, prog.uninstall_string.lower())
            if ident in seen:
                continue
            seen.add(ident)
            programs.append(prog)
    programs.sort(key=lambda p: p.name.lower())
    return programs
