//! Delete the chosen leftovers. Registry keys are exported to .reg files first.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::{Kind, Leftover};
use crate::safety;
use crate::scanner::Environment;
use crate::system::System;

#[derive(Clone, Debug)]
pub struct CleanResult {
    pub item: Leftover,
    pub ok: bool,
    pub message: String,
}

/// UTC "YYYYMMDD-HHMMSS" without pulling in a date crate.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn make_backup_dir(base: &Path, app_name: &str) -> PathBuf {
    let safe: String = app_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .take(40)
        .collect();
    let dir = base.join(format!("{}-{safe}", timestamp()));
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn clean(
    items: &[Leftover],
    env: &Environment,
    sys: &dyn System,
    backup_dir: &Path,
    log: &dyn Fn(&str),
) -> Vec<CleanResult> {
    let mut sorted: Vec<&Leftover> = items.iter().collect();
    sorted.sort_by_key(|i| i.kind.order());
    let mut results = Vec::new();
    let mut counter = 0;
    for it in sorted {
        let outcome: Result<String, String> = match it.kind {
            Kind::Service => sys.delete_service(&it.target).map(|_| "removed".into()),
            Kind::Task => sys.delete_task(&it.target).map(|_| "removed".into()),
            Kind::RegKey | Kind::RegValue => match &it.key {
                None => Err("no registry key".into()),
                Some(k) if it.kind == Kind::RegKey && safety::is_protected_key(k) => {
                    Err("refusing to touch a protected registry location".into())
                }
                Some(k) => {
                    counter += 1;
                    let _ = sys.reg_export(k, &backup_dir.join(format!("{counter:03}.reg"))); // may already be gone
                    if it.kind == Kind::RegKey {
                        sys.reg_delete_key(k)
                    } else {
                        sys.reg_delete_value(k, &it.value_name)
                    }
                    .map(|_| "removed".into())
                }
            },
            Kind::File | Kind::Dir => {
                if safety::is_protected_path(&it.target, &env.protected) {
                    Err("refusing to delete a protected location".into())
                } else {
                    let r = if fs::symlink_metadata(&it.target).is_ok() {
                        sys.delete_path(&it.target)
                    } else {
                        Ok("removed".into())
                    };
                    if r.is_ok() {
                        prune_empty_parent(&it.target, env);
                    }
                    r
                }
            }
        };
        match outcome {
            Ok(note) => {
                if note == "removed" {
                    log(&format!("  removed  {}", it.label()));
                } else {
                    log(&format!("  {note}: {}", it.label()));
                }
                results.push(CleanResult { item: it.clone(), ok: true, message: note });
            }
            Err(e) => {
                log(&format!("  FAILED   {}: {e}", it.label()));
                results.push(CleanResult { item: it.clone(), ok: false, message: e });
            }
        }
    }
    results
}

/// Remove e.g. an empty `Program Files\Adobe` after its last app folder went.
fn prune_empty_parent(path: &str, env: &Environment) {
    let Some(parent) = Path::new(path).parent() else { return };
    let parent_s = parent.to_string_lossy();
    let inside_root = env
        .file_roots
        .iter()
        .chain(&env.shortcut_roots)
        .any(|r| safety::is_within(&parent_s, r) && safety::norm(&parent_s) != safety::norm(r));
    let empty = fs::read_dir(parent).map(|mut rd| rd.next().is_none()).unwrap_or(false);
    if inside_root && empty && !safety::is_protected_path(&parent_s, &env.protected) {
        let _ = fs::remove_dir(parent);
    }
}
