"""The whole 'remove this program' flow in one place, shared by GUI and CLI."""
from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Callable, List, Optional

from . import cleaner, uninstall
from .model import HIGH, Leftover, Program
from .profiles import find_profile
from .scanner import Environment, scan_leftovers
from .system import System



def backup_base() -> str:
    return os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), "AppRemover", "backups")


Chooser = Callable[[List[Leftover]], List[Leftover]]


@dataclass
class Options:
    silent: bool = True
    force: bool = False  # skip the program's own uninstaller
    include_medium: bool = False
    dry_run: bool = False
    timeout: int = 900


@dataclass
class Report:
    uninstaller_ran: bool = False
    uninstaller_ok: bool = False
    found: List[Leftover] = field(default_factory=list)
    results: List[cleaner.Result] = field(default_factory=list)
    skipped: List[Leftover] = field(default_factory=list)


def default_chooser(include_medium: bool) -> Chooser:
    return lambda items: [i for i in items if i.confidence == HIGH or include_medium]


def remove_program(
    program: Program,
    env: Environment,
    system: System,
    options: Options,
    log: Callable[[str], None],
    choose: Optional[Chooser] = None,
    backup_base: str = "",
) -> Report:
    report = Report()
    profile = find_profile(program)

    if not options.dry_run:
        log("Stopping the program's processes...")
        if program.install_location:
            system.kill_processes_in(program.install_location)
        if profile:
            system.kill_process_names(profile.processes)

    if options.force:
        log("Force mode: skipping the program's own uninstaller.")
    else:
        command, note = uninstall.build_command(program, options.silent)
        if not uninstall.uninstaller_available(command, os.path.exists):
            why = "no uninstaller registered" if not command else "uninstaller file is missing"
            log(f"Uninstaller unavailable ({why}). Removing it manually instead.")
        elif options.dry_run:
            log(f"[dry run] would run ({note}): {command}")
        else:
            log(f"Running uninstaller ({note})...")
            report.uninstaller_ran = True
            ok, msg = uninstall.run_command(command, options.timeout)
            report.uninstaller_ok = ok
            log(("  " if ok else "  WARNING: ") + msg)
            if ok:
                uninstall.wait_until_gone(lambda: env.registry.exists(program.key), 20)
            if not ok:
                log("  Continuing with manual removal.")

    log("Scanning for leftovers...")
    report.found = scan_leftovers(program, env)
    chooser = choose or default_chooser(options.include_medium)
    selected = chooser(report.found)
    report.skipped = [i for i in report.found if i not in selected]
    log(f"Found {len(report.found)} item(s); removing {len(selected)}.")

    if options.dry_run:
        for i in report.found:
            log(f"  [{i.confidence}] {i.kind}: {i.label()}  ({i.reason})")
        return report

    if selected:
        backup_dir = cleaner.make_backup_dir(backup_base, program.name) if backup_base else os.path.join(os.getcwd(), "backup")
        os.makedirs(backup_dir, exist_ok=True)
        report.results = cleaner.clean(selected, env, system, backup_dir, log)
        if any(r.item.kind in ("regkey", "regvalue") for r in report.results):
            log(f"Registry backup saved in: {backup_dir}")
    failed = [r for r in report.results if not r.ok]
    log(f"Done. {len(report.results) - len(failed)} removed, {len(failed)} failed, {len(report.skipped)} left alone.")
    return report
