"""Side-effecting operations. File operations work everywhere; the registry,
service, task and process helpers shell out to Windows tools. Tests substitute
a recording subclass."""
from __future__ import annotations

import os
import shutil
import stat
import subprocess
import sys
from typing import List, Tuple

from .model import RegKey

IS_WINDOWS = sys.platform == "win32"
_NOWIN = getattr(subprocess, "CREATE_NO_WINDOW", 0)


class System:
    # -- helpers ---------------------------------------------------------
    def run(self, args: List[str], timeout: int = 120) -> Tuple[int, str]:
        try:
            p = subprocess.run(
                args, capture_output=True, text=True, timeout=timeout,
                creationflags=_NOWIN if IS_WINDOWS else 0,
            )
        except (OSError, subprocess.TimeoutExpired) as e:
            return 1, str(e)
        return p.returncode, (p.stdout + p.stderr).strip()

    # -- processes -------------------------------------------------------
    def kill_processes_in(self, directory: str) -> None:
        """Force-stop every process whose executable lives under ``directory``."""
        if not IS_WINDOWS or not directory:
            return
        d = directory.rstrip("\\/").replace("'", "''")
        script = (
            f"$d='{d}\\'; Get-Process | Where-Object {{ $_.Path -and "
            "$_.Path.StartsWith($d,[StringComparison]::OrdinalIgnoreCase) }} | "
            "Stop-Process -Force -ErrorAction SilentlyContinue"
        )
        self.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script])

    def kill_process_names(self, names: List[str]) -> None:
        if not IS_WINDOWS:
            return
        for n in names:
            self.run(["taskkill", "/F", "/T", "/IM", n])

    # -- services / tasks ------------------------------------------------
    def delete_service(self, name: str) -> None:
        self.run(["sc", "stop", name])
        code, out = self.run(["sc", "delete", name])
        if code != 0 and "1060" not in out:  # 1060 = service does not exist
            raise OSError(out or f"sc delete failed ({code})")

    def delete_task(self, name: str) -> None:
        code, out = self.run(["schtasks", "/Delete", "/TN", name, "/F"])
        if code != 0:
            raise OSError(out or f"schtasks failed ({code})")

    # -- registry --------------------------------------------------------
    @staticmethod
    def _reg_args(key: RegKey) -> List[str]:
        return [f"{key.hive}\\{key.path}"]

    def reg_export(self, key: RegKey, file: str) -> None:
        code, out = self.run(["reg", "export", *self._reg_args(key), file, "/y", f"/reg:{key.view}"])
        if code != 0:
            raise OSError(out or "reg export failed")

    def reg_delete_key(self, key: RegKey) -> None:
        code, out = self.run(["reg", "delete", *self._reg_args(key), "/f", f"/reg:{key.view}"])
        if code != 0:
            raise OSError(out or "reg delete failed")

    def reg_delete_value(self, key: RegKey, name: str) -> None:
        code, out = self.run(["reg", "delete", *self._reg_args(key), "/v", name, "/f", f"/reg:{key.view}"])
        if code != 0:
            raise OSError(out or "reg delete failed")

    # -- files -----------------------------------------------------------
    @staticmethod
    def _force_writable(func, path, _exc) -> None:
        try:
            os.chmod(path, stat.S_IWRITE)
            func(path)
        except OSError:
            raise

    def _remove(self, path: str) -> None:
        if os.path.isdir(path) and not os.path.islink(path):
            shutil.rmtree(path, onerror=self._force_writable)
        else:
            try:
                os.remove(path)
            except PermissionError:
                os.chmod(path, stat.S_IWRITE)
                os.remove(path)

    def delete_path(self, path: str) -> str:
        """Delete a file or folder. Returns a note; raises OSError if it can't be removed."""
        try:
            self._remove(path)
            return "deleted"
        except OSError as first:
            if not IS_WINDOWS:
                raise
            # Take ownership / grant access, retry, then fall back to delete-on-reboot.
            self.run(["takeown", "/F", path, "/R", "/D", "Y"])
            self.run(["icacls", path, "/grant", "*S-1-5-32-544:F", "/T", "/C", "/Q"])
            try:
                self._remove(path)
                return "deleted (after taking ownership)"
            except OSError:
                if self._schedule_reboot_delete(path):
                    return "in use: will be deleted at next restart"
                raise first

    def _schedule_reboot_delete(self, path: str) -> bool:
        import ctypes

        MOVEFILE_DELAY_UNTIL_REBOOT = 0x4
        ok = False
        targets = []
        if os.path.isdir(path):
            for root, dirs, files in os.walk(path, topdown=False):
                targets += [os.path.join(root, f) for f in files]
                targets += [os.path.join(root, d) for d in dirs]
        targets.append(path)
        for t in targets:
            ok = bool(ctypes.windll.kernel32.MoveFileExW(t, None, MOVEFILE_DELAY_UNTIL_REBOOT)) or ok
        return ok
