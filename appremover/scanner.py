"""Find what an app leaves behind. Matching is deliberately conservative:
only things whose name equals the app's name (HIGH) or merely starts with it
(MEDIUM) are reported, and a publisher folder/key (e.g. ``Adobe``) is never
reported itself - only searched inside."""
from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Dict, List, Set

from . import safety
from .model import DIR, FILE, HIGH, MEDIUM, REGKEY, REGVALUE, SERVICE, TASK, Leftover, Program, RegKey
from .names import basename_norm, expand_vars, normalize, publisher_variants
from .profiles import find_profile
from .registry import RegistryBackend

RESERVED_KEYS = {
    "microsoft", "classes", "policies", "wow6432node", "windows", "clients",
    "registeredapplications", "intel", "nvidia", "google", "mozilla",
}
RUN_PATHS = [
    r"Software\Microsoft\Windows\CurrentVersion\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
]
SERVICES_PATH = r"SYSTEM\CurrentControlSet\Services"


@dataclass
class Environment:
    registry: RegistryBackend
    file_roots: List[str] = field(default_factory=list)  # dirs holding per-app folders
    shortcut_roots: List[str] = field(default_factory=list)  # Start menu / Desktop
    tasks_dir: str = ""
    protected: List[str] = field(default_factory=list)
    envvars: Dict[str, str] = field(default_factory=dict)

    @classmethod
    def from_os(cls, registry: RegistryBackend) -> "Environment":
        e = os.environ
        g = lambda k: e.get(k, "")  # noqa: E731
        sysroot = g("SystemRoot") or r"C:\Windows"
        pf, pf86, pd = g("ProgramFiles"), g("ProgramFiles(x86)"), g("ProgramData")
        appdata, local, home = g("APPDATA"), g("LOCALAPPDATA"), g("USERPROFILE")
        public = g("PUBLIC")
        roots = [pf, pf86, pd, appdata, local, os.path.join(local, "Programs") if local else "",
                 os.path.join(home, "AppData", "LocalLow") if home else ""]
        shortcuts = [
            os.path.join(pd, "Microsoft", "Windows", "Start Menu", "Programs") if pd else "",
            os.path.join(appdata, "Microsoft", "Windows", "Start Menu", "Programs") if appdata else "",
            os.path.join(public, "Desktop") if public else "",
            os.path.join(home, "Desktop") if home else "",
        ]
        protected = [
            sysroot, pf, pf86, pd, appdata, local, home, public,
            os.path.dirname(home) if home else "",
            os.path.join(pf, "Common Files") if pf else "",
            os.path.join(pf86, "Common Files") if pf86 else "",
            os.path.join(pf, "WindowsApps") if pf else "",
            os.path.join(local, "Programs") if local else "",
            os.path.join(home, "Desktop") if home else "",
            os.path.join(home, "Documents") if home else "",
            os.path.join(home, "Downloads") if home else "",
        ] + shortcuts
        return cls(
            registry=registry,
            file_roots=[r for r in roots if r],
            shortcut_roots=[r for r in shortcuts if r],
            tasks_dir=os.path.join(sysroot, "System32", "Tasks"),
            protected=[p for p in protected if p],
            envvars=dict(os.environ),
        )


@dataclass
class _Info:
    names: Set[str]  # exact-match candidates
    stripped: Set[str]  # name minus publisher prefix (only trusted inside publisher dirs)
    publishers: Set[str]
    location: str


def _info(program: Program) -> _Info:
    name = normalize(program.name)
    pubs = publisher_variants(program.publisher)
    names = {name} if len(name) >= 4 else set()
    loc_name = basename_norm(program.install_location) if program.install_location else ""
    if len(loc_name) >= 4:
        names.add(loc_name)
    stripped: Set[str] = set()
    for p in pubs:
        if name.startswith(p) and len(name) - len(p) >= 4:
            stripped.add(name[len(p):])
    return _Info(names, stripped, pubs, program.install_location)


def _classify(n: str, info: _Info, inside_publisher: bool) -> str:
    """Return HIGH / MEDIUM / '' for a normalised child name."""
    if not n:
        return ""
    if n in info.names:
        return HIGH
    if inside_publisher and n in info.stripped:
        return MEDIUM
    for c in info.names | (info.stripped if inside_publisher else set()):
        if len(c) >= 6 and n.startswith(c):
            return MEDIUM
    return ""


def _strip_ext(name: str) -> str:
    return re.sub(r"\.(lnk|url)$", "", name, flags=re.I)


def _listdir(path: str) -> List[str]:
    try:
        return sorted(os.listdir(path))
    except OSError:
        return []


def _scan_fs(env: Environment, info: _Info, out: List[Leftover]) -> None:
    for root in env.file_roots + env.shortcut_roots:
        if not os.path.isdir(root):
            continue
        for entry in _listdir(root):
            full = os.path.join(root, entry)
            n = normalize(_strip_ext(entry))
            conf = _classify(n, info, False)
            if conf:
                out.append(_fs_item(full, conf, "name matches app"))
            elif n in info.publishers and os.path.isdir(full):
                for sub in _listdir(full):
                    sc = _classify(normalize(_strip_ext(sub)), info, True)
                    if sc:
                        out.append(_fs_item(os.path.join(full, sub), sc, f"inside publisher folder '{entry}'"))


def _fs_item(path: str, conf: str, reason: str) -> Leftover:
    return Leftover(DIR if os.path.isdir(path) else FILE, path, reason, conf)


def _scan_registry(env: Environment, info: _Info, out: List[Leftover]) -> None:
    reg = env.registry
    for hive, views in (("HKLM", (64, 32)), ("HKCU", (64,))):
        for view in views:
            base = RegKey(hive, "Software", view)
            for sub in reg.subkeys(base):
                n = normalize(sub)
                if n in RESERVED_KEYS:
                    continue
                key = base.child(sub)
                if n in info.names:
                    out.append(Leftover(REGKEY, key.path, "key name matches app", HIGH, key))
                elif n in info.publishers:
                    for child in reg.subkeys(key):
                        c = _classify(normalize(child), info, True)
                        if c:
                            out.append(Leftover(REGKEY, f"{key.path}\\{child}", f"inside publisher key '{sub}'", c, key.child(child)))
    # Run / RunOnce entries pointing into the install dir
    for hive, views in (("HKLM", (64, 32)), ("HKCU", (64,))):
        for view in views:
            for rp in RUN_PATHS:
                rk = RegKey(hive, rp, view)
                for vname, data in reg.values(rk).items():
                    if not isinstance(data, str):
                        continue
                    if _points_into(data, info.location) or _classify(normalize(vname), info, False) == HIGH:
                        out.append(Leftover(REGVALUE, rp, "startup entry", HIGH, rk, vname))


def _points_into(text: str, location: str) -> bool:
    return len(location) >= 6 and location.lower().rstrip("\\/") in text.lower()


def _scan_services(env: Environment, info: _Info, profile_services: List[str], out: List[Leftover]) -> None:
    reg = env.registry
    base = RegKey("HKLM", SERVICES_PATH, 64)
    wanted = {s.lower() for s in profile_services}
    for svc in reg.subkeys(base):
        vals = reg.values(base.child(svc))
        image = vals.get("ImagePath")
        if svc.lower() in wanted:
            out.append(Leftover(SERVICE, svc, "known service of this app", HIGH))
        elif isinstance(image, str) and _points_into(image, info.location):
            out.append(Leftover(SERVICE, svc, "service runs from the install folder", HIGH))


def _scan_tasks(env: Environment, info: _Info, out: List[Leftover]) -> None:
    if not env.tasks_dir or not os.path.isdir(env.tasks_dir):
        return
    for dirpath, _dirs, files in os.walk(env.tasks_dir):
        for f in files:
            full = os.path.join(dirpath, f)
            rel = os.path.relpath(full, env.tasks_dir)
            task = "\\" + rel.replace(os.sep, "\\")
            conf = _classify(normalize(f), info, False)
            hit = ""
            if conf == HIGH:
                hit = "task name matches app"
            elif info.location and len(info.location) >= 6:
                try:
                    raw = open(full, "rb").read()
                except OSError:
                    continue
                text = raw.decode("utf-16", "ignore") if raw[:2] in (b"\xff\xfe", b"\xfe\xff") else raw.decode("utf-8", "ignore")
                if _points_into(text, info.location):
                    hit, conf = "scheduled task runs from the install folder", HIGH
            if hit:
                out.append(Leftover(TASK, task, hit, conf))
            elif conf == MEDIUM:
                out.append(Leftover(TASK, task, "task name resembles app", MEDIUM))


def scan_leftovers(program: Program, env: Environment) -> List[Leftover]:
    info = _info(program)
    profile = find_profile(program)
    out: List[Leftover] = []

    if program.key and env.registry.exists(program.key):
        out.append(Leftover(REGKEY, program.key.path, "Add/Remove Programs entry", HIGH, program.key))

    loc = program.install_location
    if loc and os.path.exists(loc):
        out.append(_fs_item(loc, HIGH, "install folder"))
    if profile:
        for raw in profile.paths:
            p = expand_vars(raw, env.envvars)
            if "%" not in p and os.path.exists(p):
                out.append(_fs_item(p, HIGH, f"known {profile.name} location"))

    _scan_fs(env, info, out)
    _scan_registry(env, info, out)
    _scan_services(env, info, profile.services if profile else [], out)
    _scan_tasks(env, info, out)

    return _finalize(out, env)


def _finalize(items: List[Leftover], env: Environment) -> List[Leftover]:
    seen = set()
    unique: List[Leftover] = []
    for it in items:
        if it.kind in (FILE, DIR) and safety.is_protected_path(it.target, env.protected):
            continue
        if it.kind in (REGKEY,) and it.key is not None and safety.is_protected_key(it.key):
            continue
        ident = (it.kind, it.label().lower())
        if ident in seen:
            continue
        seen.add(ident)
        unique.append(it)
    # drop paths nested inside another path that is already listed
    dirs = [os.path.normcase(os.path.normpath(i.target)) for i in unique if i.kind == DIR]
    final = []
    for it in unique:
        if it.kind in (FILE, DIR):
            me = os.path.normcase(os.path.normpath(it.target))
            if any(d != me and me.startswith(d + os.sep) for d in dirs):
                continue
        final.append(it)
    order = {SERVICE: 0, TASK: 1, REGVALUE: 2, REGKEY: 3, DIR: 4, FILE: 5}
    final.sort(key=lambda i: (i.confidence != HIGH, order.get(i.kind, 9), i.label().lower()))
    return final
