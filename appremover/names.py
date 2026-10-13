"""Name normalisation used to match an app against folders / registry keys."""
from __future__ import annotations

import os
import re

_ARCH = re.compile(r"\(?\b(x64|x86|amd64|64-?bit|32-?bit)\b\)?", re.I)
_VERSION = re.compile(r"\bv?\d+(\.\d+)+\b", re.I)
_CORP_SUFFIX = re.compile(
    r"\b(inc|incorporated|corp|corporation|ltd|limited|llc|gmbh|co|company|s\.?a\.?)\b\.?",
    re.I,
)
GENERIC_DIRS = {"bin", "app", "application", "program", "programs", "x64", "x86", "win64", "win32"}


def normalize(text: str) -> str:
    text = _ARCH.sub(" ", text or "")
    text = _VERSION.sub(" ", text)
    return re.sub(r"[^a-z0-9]", "", text.lower())


def publisher_variants(publisher: str) -> set:
    """'Adobe Systems Incorporated' -> {'adobesystemsincorporated', 'adobesystems', 'adobe'}."""
    out = set()
    full = normalize(publisher)
    if len(full) >= 3:
        out.add(full)
    stripped = normalize(_CORP_SUFFIX.sub(" ", publisher or ""))
    if len(stripped) >= 3:
        out.add(stripped)
    words = re.findall(r"[A-Za-z0-9]+", publisher or "")
    if words and len(words[0]) >= 3:
        out.add(words[0].lower())
    return out


def expand_vars(text: str, env: dict) -> str:
    """Expand %VAR% using ``env`` (case-insensitive); unknown vars are left alone."""
    lowered = {k.lower(): v for k, v in env.items()}
    return re.sub(r"%([^%]+)%", lambda m: lowered.get(m.group(1).lower(), m.group(0)), text)


def basename_norm(path: str) -> str:
    base = os.path.basename(path.replace("\\", "/").rstrip("/\\"))
    n = normalize(base)
    return "" if n in GENERIC_DIRS else n
