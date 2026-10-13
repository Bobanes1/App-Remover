from __future__ import annotations

from dataclasses import dataclass
from typing import Optional


@dataclass(frozen=True)
class RegKey:
    """A registry key. ``view`` is 64 or 32 (the WOW64 registry view)."""

    hive: str  # "HKLM" or "HKCU"
    path: str
    view: int = 64

    def __str__(self) -> str:
        return f"{self.hive}\\{self.path}"

    def child(self, name: str) -> "RegKey":
        return RegKey(self.hive, f"{self.path}\\{name}", self.view)


@dataclass
class Program:
    name: str
    key: RegKey
    publisher: str = ""
    version: str = ""
    install_location: str = ""
    uninstall_string: str = ""
    quiet_uninstall_string: str = ""
    size_kb: int = 0
    install_date: str = ""
    windows_installer: bool = False

    @property
    def key_name(self) -> str:
        return self.key.path.rsplit("\\", 1)[-1]


# Leftover kinds
FILE, DIR, REGKEY, REGVALUE, SERVICE, TASK = (
    "file", "dir", "regkey", "regvalue", "service", "task",
)

HIGH, MEDIUM = "high", "medium"


@dataclass
class Leftover:
    kind: str
    target: str  # path, registry key path, service name or task name
    reason: str = ""
    confidence: str = HIGH
    key: Optional[RegKey] = None  # for regkey / regvalue
    value_name: str = ""  # for regvalue

    def label(self) -> str:
        if self.kind in (REGKEY, REGVALUE) and self.key is not None:
            tail = f" [{self.value_name}]" if self.kind == REGVALUE else ""
            return f"{self.key}{tail}"
        return self.target
