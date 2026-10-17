//! The whole "remove this program" flow, shared by the GUI and the command line.
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cleaner::{self, CleanResult};
use crate::model::{Confidence, Kind, Leftover, Program};
use crate::profiles::find_profile;
use crate::registry::Registry;
use crate::scanner::{own_install_folder, scan_leftovers, Environment};
use crate::system::System;
use crate::uninstall;

#[derive(Clone, Debug)]
pub struct Options {
    /// Try to make the uninstaller run without its wizard.
    pub silent: bool,
    /// Skip the program's own uninstaller.
    pub force: bool,
    pub include_medium: bool,
    /// Only report what would happen.
    pub dry_run: bool,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options { silent: true, force: false, include_medium: false, dry_run: false, timeout: Duration::from_secs(900) }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub uninstaller_ran: bool,
    pub uninstaller_ok: bool,
    pub found: Vec<Leftover>,
    pub results: Vec<CleanResult>,
    pub skipped: usize,
}

pub fn backup_base() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("AppRemover").join("backups")
}

/// Default choice: everything High, plus Medium if asked for.
pub fn default_choice(items: &[Leftover], include_medium: bool) -> Vec<usize> {
    (0..items.len()).filter(|&i| include_medium || items[i].confidence == Confidence::High).collect()
}

/// `choose` gets all found leftovers and returns the indices to delete.
#[allow(clippy::too_many_arguments)]
pub fn remove_program(
    program: &Program,
    env: &Environment,
    reg: &dyn Registry,
    sys: &dyn System,
    opts: &Options,
    log: &dyn Fn(&str),
    choose: &dyn Fn(&[Leftover]) -> Vec<usize>,
    backup_base: &Path,
) -> Report {
    let mut report = Report::default();
    let profile = find_profile(program);

    let own_folder = own_install_folder(program, env, reg);
    if own_folder.is_none() && !program.install_location.trim().is_empty() {
        log(&format!(
            "Note: its install location '{}' is a system or shared folder; it will not be touched.",
            program.install_location.trim()
        ));
    }
    if !opts.dry_run {
        log("Stopping the program's processes...");
        if let Some(folder) = &own_folder {
            sys.kill_processes_in(folder);
        }
        if let Some(p) = profile {
            sys.kill_process_names(p.processes);
        }
    }

    if opts.force {
        log("Force mode: skipping the program's own uninstaller.");
    } else {
        let (command, note) = uninstall::build_command(program, opts.silent);
        if !uninstall::uninstaller_available(command.as_deref(), |p| Path::new(p).exists()) {
            let why = if command.is_none() { "no uninstaller registered" } else { "uninstaller file is missing" };
            log(&format!("Uninstaller unavailable ({why}). Removing it manually instead."));
        } else if let Some(command) = command {
            if opts.dry_run {
                log(&format!("[dry run] would run ({note}): {command}"));
            } else {
                log(&format!("Running uninstaller ({note})..."));
                report.uninstaller_ran = true;
                let (ok, msg) = uninstall::run_command(&command, opts.timeout);
                report.uninstaller_ok = ok;
                log(&format!("  {}{msg}", if ok { "" } else { "WARNING: " }));
                if ok {
                    uninstall::wait_until_gone(|| reg.exists(&program.key), Duration::from_secs(20));
                } else {
                    log("  Continuing with manual removal.");
                }
            }
        }
    }

    log("Scanning for leftovers...");
    report.found = scan_leftovers(program, env, reg);
    let mut picked = choose(&report.found);
    picked.sort_unstable();
    picked.dedup();
    picked.retain(|&i| i < report.found.len());
    report.skipped = report.found.len() - picked.len();
    log(&format!("Found {} item(s); removing {}.", report.found.len(), picked.len()));

    if opts.dry_run {
        for i in &report.found {
            let conf = if i.confidence == Confidence::High { "high" } else { "medium" };
            log(&format!("  [{conf}] {}: {}  ({})", i.kind.as_str(), i.label(), i.reason));
        }
        log("Dry run: nothing was changed.");
        return report;
    }

    if !picked.is_empty() {
        let selected: Vec<Leftover> = picked.iter().map(|&i| report.found[i].clone()).collect();
        let backup_dir = cleaner::make_backup_dir(backup_base, &program.name);
        report.results = cleaner::clean(&selected, env, sys, &backup_dir, log);
        if selected.iter().any(|i| matches!(i.kind, Kind::RegKey | Kind::RegValue)) {
            log(&format!("Registry backup saved in: {}", backup_dir.display()));
        }
    }
    let failed = report.results.iter().filter(|r| !r.ok).count();
    log(&format!("Done. {} removed, {failed} failed, {} left alone.", report.results.len() - failed, report.skipped));
    report
}
