//! Which drive a program lives on, and how big its folder is when the
//! registry doesn't say.
use std::fs;
use std::path::Path;

use crate::model::Program;
use crate::uninstall::split_command;

/// "d:\Games\x" -> "D:"; None for paths without a drive letter.
fn drive_letter(path: &str) -> Option<String> {
    let b = path.trim().trim_matches('"').as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| format!("{}:", (b[0] as char).to_ascii_uppercase()))
}

/// Drive ("C:") the program is installed on, judged by its install folder, then its
/// icon, then its uninstaller. Uninstallers kept under Windows are ignored, since
/// they say nothing about where the program itself lives.
pub fn program_drive(p: &Program) -> Option<String> {
    let (uninstaller, _) = split_command(&p.uninstall_string);
    let uninstaller = if uninstaller.to_lowercase().contains(":\\windows\\") { String::new() } else { uninstaller };
    for candidate in [p.install_location.as_str(), p.display_icon.as_str(), uninstaller.as_str()] {
        if let Some(drive) = drive_letter(candidate) {
            return Some(drive);
        }
    }
    None
}

/// Total size of the files below `path` in KB (rounded up). Symlinks and junctions
/// are not followed; unreadable parts are skipped.
pub fn folder_size_kb(path: &Path) -> u64 {
    let mut bytes: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue }; // does not follow symlinks
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
            }
        }
    }
    bytes.div_ceil(1024)
}
