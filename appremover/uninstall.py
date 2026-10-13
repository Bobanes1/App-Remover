"""Building and running the program's own uninstaller."""
from __future__ import annotations

import ntpath
import re
import subprocess
import sys
import time
from typing import Callable, Optional, Tuple

from .model import Program

GUID_RE = re.compile(r"\{[0-9A-Fa-f]{8}-(?:[0-9A-Fa-f]{4}-){3}[0-9A-Fa-f]{12}\}")
OK_CODES = {0, 1641, 3010}  # success / reboot initiated / reboot required
_NOWIN = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def split_command(cmd: str) -> Tuple[str, str]:
    """Split a Windows command line into (executable, argument string)."""
    cmd = cmd.strip()
    if not cmd:
        return "", ""
    if cmd[0] == '"':
        end = cmd.find('"', 1)
        if end == -1:
            return cmd[1:], ""
        return cmd[1:end], cmd[end + 1:].strip()
    m = re.match(r"^(.*?\.(?:exe|msi|bat|cmd|com))(?:\s+(.*))?$", cmd, re.I)
    if m:
        return m.group(1), (m.group(2) or "").strip()
    parts = cmd.split(None, 1)
    return parts[0], (parts[1] if len(parts) > 1 else "")


def _is_msiexec(exe: str) -> bool:
    return ntpath.basename(exe).lower() in ("msiexec", "msiexec.exe")


def build_command(program: Program, silent: bool = True) -> Tuple[Optional[str], str]:
    """Return (command line, note). Command is None if there is nothing to run."""
    if silent and program.quiet_uninstall_string:
        return program.quiet_uninstall_string, "quiet uninstall string"

    cmd = program.uninstall_string
    if not cmd:
        if program.windows_installer and GUID_RE.fullmatch(program.key_name):
            cmd = f"msiexec.exe /x {program.key_name}"
        else:
            return None, "no uninstall command registered"

    exe, args = split_command(cmd)
    if _is_msiexec(exe):
        guid = GUID_RE.search(args)
        if not guid:
            return cmd, "msiexec (as registered)"
        flags = " /qn /norestart" if silent else ""
        return f'msiexec.exe /x {guid.group(0)}{flags}', "Windows Installer"
    if silent and re.fullmatch(r"unins\d{3}\.exe", ntpath.basename(exe), re.I):
        return f'"{exe}" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART', "Inno Setup uninstaller"
    return cmd, "registered uninstaller"


def uninstaller_available(command: Optional[str], exists: Callable[[str], bool]) -> bool:
    if not command:
        return False
    exe, _ = split_command(command)
    if _is_msiexec(exe):
        return True
    return exists(exe)


def run_command(command: str, timeout: int = 900) -> Tuple[bool, str]:
    """Run an uninstall command line and wait. Returns (success, message)."""
    try:
        proc = subprocess.run(command, timeout=timeout, creationflags=_NOWIN if sys.platform == "win32" else 0)
    except subprocess.TimeoutExpired:
        return False, f"uninstaller still running after {timeout}s, moving on"
    except OSError as e:
        return False, f"could not start uninstaller: {e}"
    if proc.returncode in OK_CODES:
        return True, f"uninstaller finished (code {proc.returncode})"
    return False, f"uninstaller exited with code {proc.returncode}"


def wait_until_gone(check: Callable[[], bool], seconds: float = 20, step: float = 1.0) -> bool:
    """Some uninstallers relaunch themselves from %TEMP% and return at once; wait
    for the Uninstall registry entry to vanish. True if it vanished."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if not check():
            return True
        time.sleep(step)
    return not check()
