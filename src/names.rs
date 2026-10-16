//! Name normalisation used to match an app against folders and registry keys.
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

static ARCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\(?\b(x64|x86|amd64|64-?bit|32-?bit)\b\)?").unwrap());
static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bv?\d+(\.\d+)+\b").unwrap());
static CORP_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(inc|incorporated|corp|corporation|ltd|limited|llc|gmbh|ag|co|company|s\.?a\.?)\b\.?").unwrap()
});
static VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%([^%]+)%").unwrap());

const GENERIC_DIRS: &[&str] = &["bin", "app", "application", "program", "programs", "x64", "x86", "win64", "win32"];

/// "7-Zip 23.01 (x64)" -> "7zip"
pub fn normalize(text: &str) -> String {
    let t = ARCH.replace_all(text, " ");
    let t = VERSION.replace_all(&t, " ");
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// "Adobe Systems Incorporated" -> {"adobesystemsincorporated", "adobesystems", "adobe"}
pub fn publisher_variants(publisher: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let full = normalize(publisher);
    if full.chars().count() >= 3 {
        out.insert(full);
    }
    let stripped = normalize(&CORP_SUFFIX.replace_all(publisher, " "));
    if stripped.chars().count() >= 3 {
        out.insert(stripped);
    }
    if let Some(first) = publisher.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()) {
        if first.chars().count() >= 3 {
            out.insert(first.to_lowercase());
        }
    }
    out
}

/// Expand `%VAR%` (case-insensitive); unknown variables are left untouched.
pub fn expand_vars(text: &str, env: &HashMap<String, String>) -> String {
    let lowered: HashMap<String, &String> = env.iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
    VAR.replace_all(text, |c: &regex::Captures| match lowered.get(&c[1].to_lowercase()) {
        Some(v) => (*v).clone(),
        None => c[0].to_string(),
    })
    .into_owned()
}

/// Normalised last path component, or "" for generic names like "bin".
pub fn basename_norm(path: &str) -> String {
    let p = path.replace('\\', "/");
    let base = p.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let n = normalize(base);
    if GENERIC_DIRS.contains(&n.as_str()) {
        String::new()
    } else {
        n
    }
}
