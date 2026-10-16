//! Guards that stop the cleaner from ever touching system locations.
use std::path::{Path, MAIN_SEPARATOR};

use crate::model::RegKey;

const PROTECTED_KEY_PATHS: &[&str] = &[
    "software",
    "software\\microsoft",
    "software\\classes",
    "software\\policies",
    "software\\wow6432node",
    "software\\microsoft\\windows",
    "software\\microsoft\\windows\\currentversion",
    "software\\microsoft\\windows\\currentversion\\uninstall",
    "software\\microsoft\\windows\\currentversion\\run",
    "software\\microsoft\\windows\\currentversion\\runonce",
    "system",
    "system\\currentcontrolset",
    "system\\currentcontrolset\\services",
];

/// Canonical form for comparisons: case-insensitive with `\` on Windows, no trailing separator.
pub fn norm(p: &str) -> String {
    let mut s = if cfg!(windows) { p.replace('/', "\\").to_lowercase() } else { p.to_string() };
    while s.len() > 1 && s.ends_with(MAIN_SEPARATOR) && !(cfg!(windows) && s.len() == 3 && s.as_bytes()[1] == b':') {
        s.pop();
    }
    s
}

/// `path` equals `parent` or lies somewhere below it.
pub fn is_within(path: &str, parent: &str) -> bool {
    let (p, q) = (norm(path), norm(parent));
    if p == q {
        return true;
    }
    let mut prefix = q.trim_end_matches(MAIN_SEPARATOR).to_string();
    prefix.push(MAIN_SEPARATOR);
    p.starts_with(&prefix)
}

/// True if `path` is relative, a drive root, or is/contains a protected location.
pub fn is_protected_path(path: &str, protected: &[String]) -> bool {
    if path.trim().is_empty() {
        return true;
    }
    let p = Path::new(path);
    if !p.is_absolute() || p.parent().is_none() {
        return true;
    }
    // equal to, or an ancestor of, a protected dir (e.g. C:\ or C:\Users)
    protected.iter().filter(|x| !x.is_empty()).any(|prot| is_within(prot, path))
}

pub fn is_protected_key(key: &RegKey) -> bool {
    let low = key.path.trim_matches('\\').to_lowercase();
    PROTECTED_KEY_PATHS.contains(&low.as_str()) || !low.contains('\\')
}
