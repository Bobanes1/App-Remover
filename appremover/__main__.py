from __future__ import annotations

import argparse
import ctypes
import sys
from typing import List, Optional

from . import __version__
from .engine import Options, backup_base, remove_program
from .model import Program
from .registry import default_backend, list_programs
from .scanner import Environment
from .system import System


def is_admin() -> bool:
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def relaunch_as_admin() -> None:
    """Re-run ourselves elevated (one UAC prompt) and exit this instance."""
    if getattr(sys, "frozen", False):
        exe, params = sys.executable, sys.argv[1:]
    else:
        exe, params = sys.executable, ["-m", "appremover", *sys.argv[1:]]
    cmd = " ".join(f'"{a}"' if " " in a else a for a in params)
    ctypes.windll.shell32.ShellExecuteW(None, "runas", exe, cmd, None, 1)
    sys.exit(0)


def pick(programs: List[Program], query: str) -> Optional[Program]:
    q = query.lower()
    exact = [p for p in programs if p.name.lower() == q]
    if len(exact) == 1:
        return exact[0]
    hits = [p for p in programs if q in p.name.lower()]
    if len(hits) == 1:
        return hits[0]
    if not hits:
        print(f"No installed program matches '{query}'.")
    else:
        print(f"'{query}' is ambiguous, be more specific:")
        for p in hits:
            print(f"  {p.name} {p.version}")
    return None


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(prog="appremover", description="Fast program uninstaller with leftover cleanup.")
    ap.add_argument("--version", action="version", version=__version__)
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("gui", help="open the window (default)")
    ls = sub.add_parser("list", help="list installed programs")
    ls.add_argument("filter", nargs="?", default="")
    rm = sub.add_parser("remove", help="uninstall a program and clean up its leftovers")
    rm.add_argument("name")
    rm.add_argument("--force", action="store_true", help="don't run the program's own uninstaller")
    rm.add_argument("--interactive", action="store_true", help="don't try to make the uninstaller silent")
    rm.add_argument("--include-medium", action="store_true", help="also delete lower-confidence leftovers")
    rm.add_argument("--dry-run", action="store_true", help="only show what would be done")
    args = ap.parse_args(argv)

    if sys.platform != "win32":
        print("App Remover only runs on Windows.")
        return 2
    if not is_admin():
        relaunch_as_admin()

    reg = default_backend()
    if args.cmd in (None, "gui"):
        from .gui import run_gui
        run_gui(reg)
        return 0

    programs = list_programs(reg)
    if args.cmd == "list":
        for p in programs:
            if args.filter.lower() in p.name.lower():
                print(f"{p.name}\t{p.version}\t{p.publisher}")
        return 0

    prog = pick(programs, args.name)
    if not prog:
        return 1
    opts = Options(silent=not args.interactive, force=args.force,
                   include_medium=args.include_medium, dry_run=args.dry_run)
    report = remove_program(prog, Environment.from_os(reg), System(), opts, print, backup_base=backup_base())
    return 1 if any(not r.ok for r in report.results) else 0


if __name__ == "__main__":
    sys.exit(main())
