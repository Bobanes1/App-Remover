"""Delete the chosen leftovers (registry keys are exported to a .reg backup first)."""
from __future__ import annotations

import os
import re
import time
from dataclasses import dataclass
from typing import Callable, List, Optional

from . import safety
from .model import DIR, FILE, REGKEY, REGVALUE, SERVICE, TASK, Leftover
from .scanner import Environment
from .system import System


@dataclass
class Result:
    item: Leftover
    ok: bool
    message: str


def make_backup_dir(base: str, app_name: str) -> str:
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", app_name)[:40]
    path = os.path.join(base, f"{time.strftime('%Y%m%d-%H%M%S')}-{safe}")
    os.makedirs(path, exist_ok=True)
    return path


def clean(
    items: List[Leftover],
    env: Environment,
    system: System,
    backup_dir: str,
    log: Optional[Callable[[str], None]] = None,
) -> List[Result]:
    log = log or (lambda _m: None)
    order = {SERVICE: 0, TASK: 1, REGVALUE: 2, REGKEY: 3, FILE: 4, DIR: 5}
    results: List[Result] = []
    counter = 0
    for it in sorted(items, key=lambda i: order.get(i.kind, 9)):
        try:
            note = "removed"
            if it.kind == SERVICE:
                system.delete_service(it.target)
            elif it.kind == TASK:
                system.delete_task(it.target)
            elif it.kind in (REGKEY, REGVALUE):
                if it.key is None or (it.kind == REGKEY and safety.is_protected_key(it.key)):
                    raise PermissionError("refusing to touch a protected registry location")
                counter += 1
                bfile = os.path.join(backup_dir, f"{counter:03d}.reg")
                try:
                    system.reg_export(it.key, bfile)
                except OSError:
                    pass  # nothing to back up / key already gone
                if it.kind == REGKEY:
                    system.reg_delete_key(it.key)
                else:
                    system.reg_delete_value(it.key, it.value_name)
            elif it.kind in (FILE, DIR):
                if safety.is_protected_path(it.target, env.protected):
                    raise PermissionError("refusing to delete a protected location")
                if os.path.lexists(it.target):
                    note = system.delete_path(it.target)
                _prune_empty_parent(it.target, env, system)
            results.append(Result(it, True, note))
            log(f"  removed  {it.label()}" if note == "removed" else f"  {note}: {it.label()}")
        except OSError as e:
            results.append(Result(it, False, str(e)))
            log(f"  FAILED   {it.label()}: {e}")
    return results


def _prune_empty_parent(path: str, env: Environment, system: System) -> None:
    """Remove e.g. an empty ``Program Files\\Adobe`` after its last app folder went."""
    parent = os.path.dirname(os.path.normpath(path))
    try:
        if (
            parent
            and os.path.isdir(parent)
            and not os.listdir(parent)
            and not safety.is_protected_path(parent, env.protected)
            and any(safety.is_within(parent, r) and os.path.normcase(parent) != os.path.normcase(r)
                    for r in env.file_roots + env.shortcut_roots)
        ):
            os.rmdir(parent)
    except OSError:
        pass
