"""Guards that stop the cleaner from ever touching system locations."""
from __future__ import annotations

import os
from typing import Iterable

from .model import RegKey

PROTECTED_KEY_PATHS = {
    "software",
    "software\\microsoft",
    "software\\classes",
    "software\\policies",
    "software\\wow6432node",
    "software\\microsoft\\windows",
    "software\\microsoft\\windows\\currentversion",
    "software\\microsoft\\windows\\currentversion\\uninstall",
    "software\\microsoft\\windows\\currentversion\\run",
    "system",
    "system\\currentcontrolset",
    "system\\currentcontrolset\\services",
}


def _norm(p: str) -> str:
    return os.path.normcase(os.path.normpath(p))


def is_within(path: str, parent: str) -> bool:
    p, q = _norm(path), _norm(parent)
    return p == q or p.startswith(q.rstrip(os.sep) + os.sep)


def is_protected_path(path: str, protected: Iterable[str]) -> bool:
    """True if ``path`` is, or contains, a protected location (or is a drive root)."""
    if not path or not path.strip():
        return True
    p = _norm(path)
    if os.path.dirname(p) == p:  # filesystem / drive root
        return True
    for prot in protected:
        if not prot:
            continue
        # equal to, or an ancestor of, a protected dir (e.g. C:\ or C:\Users)
        if is_within(prot, p):
            return True
    return False


def is_protected_key(key: RegKey) -> bool:
    low = key.path.strip("\\").lower()
    return low in PROTECTED_KEY_PATHS or low.count("\\") < 1
