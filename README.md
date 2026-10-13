# App Remover

A small Revo-Uninstaller-style tool for Windows: pick a program, click **Remove**, done.
No wizard, no menus, no Python packages to install (standard library only).

## What "Remove" does

1. Stops the program's running processes (and known helper services for apps like Adobe Creative Cloud).
2. Runs the program's own uninstaller **silently** where possible (MSI `/qn`, Inno Setup `/VERYSILENT`, or the
   program's registered quiet command).
3. **If the uninstaller is missing, broken or fails** (the classic *"uninstaller is unavailable"* error), it does not stop:
   it carries on and removes the program by hand.
4. Scans for leftovers and deletes them: install folder, matching folders in Program Files / ProgramData / AppData,
   Start-menu and desktop shortcuts, registry keys, startup entries, services and scheduled tasks, and finally the
   entry in Apps & Features.
5. Registry keys are exported to `%LOCALAPPDATA%\AppRemover\backups\...` as `.reg` files before deletion.
   Files are deleted permanently (locked files are scheduled for deletion at next restart).

Matching is deliberately conservative. Items whose name equals the program's name are deleted automatically;
items that merely look similar are listed only if you tick **Review leftovers** (or pass `--include-medium` on the command
line). Publisher folders such as `Adobe` are never deleted, only searched inside, so removing Creative Cloud leaves Photoshop alone.
System folders and top-level registry keys are hard-blocked.

## Use it

Easiest: get `AppRemover.exe` from the **Actions → build → Artifacts** tab of this repo (or run `build.bat` yourself),
and double-click it. It asks for administrator rights once.

From source (Windows, Python 3.9+): `python -m appremover` (GUI) or

```
python -m appremover list adobe
python -m appremover remove "Adobe Creative Cloud" --dry-run   # show what would happen
python -m appremover remove "Adobe Creative Cloud"             # do it, no questions
python -m appremover remove "Some App" --force                 # skip its uninstaller entirely
```

## Status

The logic (program discovery, uninstall-command building, leftover matching, safety guards, cleanup) is covered by
unit tests that run on any OS (`python -m unittest discover -s tests`). The Windows calls (`winreg`, `reg`, `sc`,
`schtasks`, `taskkill`) are thin wrappers and have **not yet been exercised on a real Windows machine**: try
`--dry-run` first on anything you care about.
