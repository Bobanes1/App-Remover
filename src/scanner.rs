//! Find what an app leaves behind. Matching is deliberately conservative: only
//! things whose name equals the app's name (High) or merely starts with it (Medium)
//! are reported, and a publisher folder/key (e.g. `Adobe`) is never reported
//! itself - it is only searched inside.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, MAIN_SEPARATOR};

use crate::model::{Confidence, Hive, Kind, Leftover, Program, RegKey, RegValue};
use crate::names::{basename_norm, expand_vars, normalize, publisher_variants};
use crate::profiles::find_profile;
use crate::registry::{list_programs, Registry};
use crate::safety;

const RESERVED_KEYS: &[&str] = &[
    "microsoft",
    "classes",
    "policies",
    "wow6432node",
    "windows",
    "clients",
    "registeredapplications",
    "intel",
    "nvidia",
    "google",
    "mozilla",
];
const RUN_PATHS: &[&str] =
    &[r"Software\Microsoft\Windows\CurrentVersion\Run", r"Software\Microsoft\Windows\CurrentVersion\RunOnce"];
pub const SERVICES_PATH: &str = r"SYSTEM\CurrentControlSet\Services";

/// Where to look, and what must never be touched.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// Folders that hold per-app folders (Program Files, ProgramData, AppData...).
    pub file_roots: Vec<String>,
    /// Start menu and desktop folders.
    pub shortcut_roots: Vec<String>,
    pub tasks_dir: String,
    /// Never deleted themselves, nor any of their parents.
    pub protected: Vec<String>,
    /// Whole trees nothing may be deleted from (e.g. C:\Windows), see [`safety::is_forbidden`].
    pub forbidden: Vec<String>,
    pub envvars: HashMap<String, String>,
}

fn join(base: &str, parts: &[&str]) -> String {
    if base.is_empty() {
        return String::new();
    }
    let mut p = Path::new(base).to_path_buf();
    for part in parts {
        p.push(part);
    }
    p.to_string_lossy().into_owned()
}

impl Environment {
    pub fn from_os() -> Self {
        let g = |k: &str| std::env::var(k).unwrap_or_default();
        let sysroot = Some(g("SystemRoot")).filter(|s| !s.is_empty()).unwrap_or_else(|| r"C:\Windows".into());
        let (pf, pf86, pd) = (g("ProgramFiles"), g("ProgramFiles(x86)"), g("ProgramData"));
        let (appdata, local, home, public) = (g("APPDATA"), g("LOCALAPPDATA"), g("USERPROFILE"), g("PUBLIC"));
        let start = ["Microsoft", "Windows", "Start Menu", "Programs"];
        let file_roots = vec![
            pf.clone(),
            pf86.clone(),
            pd.clone(),
            appdata.clone(),
            local.clone(),
            join(&local, &["Programs"]),
            join(&home, &["AppData", "LocalLow"]),
        ];
        let shortcut_roots =
            vec![join(&pd, &start), join(&appdata, &start), join(&public, &["Desktop"]), join(&home, &["Desktop"])];
        let users = Path::new(&home).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let mut protected = vec![
            sysroot.clone(),
            pf.clone(),
            pf86.clone(),
            pd,
            appdata,
            local.clone(),
            home.clone(),
            public,
            users,
            join(&pf, &["Common Files"]),
            join(&pf86, &["Common Files"]),
            join(&pf, &["WindowsApps"]),
            join(&local, &["Programs"]),
            join(&home, &["Desktop"]),
            join(&home, &["Documents"]),
            join(&home, &["Downloads"]),
            // personal folders and temp folders: never deleted as a whole
            join(&local, &["Temp"]),
            g("TEMP"),
            join(&home, &["Pictures"]),
            join(&home, &["Music"]),
            join(&home, &["Videos"]),
            join(&home, &["Favorites"]),
            join(&home, &["Saved Games"]),
            join(&home, &["OneDrive"]),
            g("OneDrive"),
        ];
        protected.extend(shortcut_roots.iter().cloned());
        let mut forbidden = vec![
            sysroot.clone(),
            join(&g("ProgramData"), &["Microsoft"]),
            join(&pf, &["WindowsApps"]),
            join(&pf, &["Windows Defender"]),
            join(&pf86, &["Windows Defender"]),
            join(&pf, &["Windows Defender Advanced Threat Protection"]),
            join(&pf, &["Common Files", "microsoft shared"]),
            join(&pf86, &["Common Files", "microsoft shared"]),
            join(&local, &["Microsoft"]),
            join(&local, &["Packages"]),
            join(&g("APPDATA"), &["Microsoft"]),
        ];
        // App Remover's own folder: it must never delete itself.
        if let Some(dir) =
            std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_string_lossy().into_owned()))
        {
            forbidden.push(dir);
        }
        let clean = |v: Vec<String>| v.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>();
        Environment {
            file_roots: clean(file_roots),
            shortcut_roots: clean(shortcut_roots),
            tasks_dir: join(&sysroot, &["System32", "Tasks"]),
            protected: clean(protected),
            forbidden: clean(forbidden),
            envvars: std::env::vars().collect(),
        }
    }
}

impl Environment {
    /// May App Remover delete this path (or stop processes running from it)?
    pub fn may_touch(&self, path: &str) -> bool {
        !safety::is_protected_path(path, &self.protected) && !safety::is_forbidden(path, &self.forbidden)
    }
}

/// The program's install folder, if it is really its own: not a system or protected
/// folder, and not a shared parent that other installed programs live in (e.g. an
/// entry pointing at `D:\Games` while `D:\Games\Foo` is another program).
/// Only this folder is deleted, used to recognise its services, tasks and startup
/// entries, and has its processes stopped.
pub fn own_install_folder(program: &Program, env: &Environment, reg: &dyn Registry) -> Option<String> {
    let loc = program.install_location.trim().trim_end_matches(['\\', '/']);
    if loc.is_empty() || !Path::new(loc).is_absolute() || !env.may_touch(loc) {
        return None;
    }
    let shared = list_programs(reg).iter().any(|o| {
        o.key != program.key
            && !o.install_location.trim().is_empty()
            && safety::is_within(o.install_location.trim(), loc)
            && safety::norm(o.install_location.trim()) != safety::norm(loc)
    });
    (!shared).then(|| loc.to_string())
}

struct Info {
    names: HashSet<String>,
    /// Name minus publisher prefix: only trusted inside a publisher folder.
    stripped: HashSet<String>,
    publishers: HashSet<String>,
    /// The program's own install folder, or "" if it has none we can trust.
    location: String,
    /// Another installed program has the same name once versions are stripped
    /// (e.g. "Python 3.11" and "Python 3.12"): name matches are then never certain.
    ambiguous: bool,
}

impl Info {
    fn name_confidence(&self) -> Confidence {
        if self.ambiguous {
            Confidence::Medium
        } else {
            Confidence::High
        }
    }
}

fn info(program: &Program, location: Option<String>, reg: &dyn Registry) -> Info {
    let name = normalize(&program.name);
    let publishers = publisher_variants(&program.publisher);
    let mut names = HashSet::new();
    if name.len() >= 4 {
        names.insert(name.clone());
    }
    let location = location.unwrap_or_default();
    let loc_name = basename_norm(&location);
    if loc_name.len() >= 4 {
        names.insert(loc_name);
    }
    let stripped = publishers
        .iter()
        .filter_map(|p| name.strip_prefix(p.as_str()))
        .filter(|rest| rest.len() >= 4)
        .map(str::to_string)
        .collect();
    let ambiguous = list_programs(reg).iter().any(|o| o.key != program.key && normalize(&o.name) == name);
    Info { names, stripped, publishers, location, ambiguous }
}

fn classify(n: &str, info: &Info, inside_publisher: bool) -> Option<Confidence> {
    if n.is_empty() {
        return None;
    }
    if info.names.contains(n) {
        return Some(info.name_confidence());
    }
    if inside_publisher && info.stripped.contains(n) {
        return Some(Confidence::Medium);
    }
    let extra = if inside_publisher { Some(&info.stripped) } else { None };
    let prefix_hit =
        info.names.iter().chain(extra.into_iter().flatten()).any(|c| c.len() >= 6 && n.starts_with(c.as_str()));
    prefix_hit.then_some(Confidence::Medium)
}

fn strip_ext(name: &str) -> &str {
    let low = name.to_lowercase();
    if low.ends_with(".lnk") || low.ends_with(".url") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

fn list_dir(path: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(path)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn is_dir(path: &str) -> bool {
    fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn fs_item(path: String, conf: Confidence, reason: impl Into<String>) -> Leftover {
    let kind = if is_dir(&path) { Kind::Dir } else { Kind::File };
    Leftover::new(kind, path, reason, conf)
}

fn scan_fs(env: &Environment, info: &Info, out: &mut Vec<Leftover>) {
    for root in env.file_roots.iter().chain(&env.shortcut_roots) {
        if !is_dir(root) {
            continue;
        }
        for entry in list_dir(root) {
            let full = join(root, &[&entry]);
            let n = normalize(strip_ext(&entry));
            if let Some(c) = classify(&n, info, false) {
                out.push(fs_item(full, c, "name matches app"));
            } else if info.publishers.contains(&n) && is_dir(&full) {
                for sub in list_dir(&full) {
                    if let Some(c) = classify(&normalize(strip_ext(&sub)), info, true) {
                        out.push(fs_item(join(&full, &[&sub]), c, format!("inside publisher folder '{entry}'")));
                    }
                }
            }
        }
    }
}

fn points_into(text: &str, location: &str) -> bool {
    let loc = location.trim_end_matches(['\\', '/']).to_lowercase();
    loc.len() >= 6 && text.to_lowercase().contains(&loc)
}

const VIEWS: &[(Hive, &[u32])] = &[(Hive::Hklm, &[64, 32]), (Hive::Hkcu, &[64])];

fn scan_registry(reg: &dyn Registry, info: &Info, out: &mut Vec<Leftover>) {
    for &(hive, views) in VIEWS {
        for &view in views {
            let base = RegKey::new(hive, "Software", view);
            for sub in reg.subkeys(&base) {
                let n = normalize(&sub);
                if RESERVED_KEYS.contains(&n.as_str()) {
                    continue;
                }
                let key = base.child(&sub);
                if info.names.contains(&n) {
                    out.push(Leftover::reg_key(key, "key name matches app", info.name_confidence()));
                } else if info.publishers.contains(&n) {
                    for child in reg.subkeys(&key) {
                        if let Some(c) = classify(&normalize(&child), info, true) {
                            out.push(Leftover::reg_key(key.child(&child), format!("inside publisher key '{sub}'"), c));
                        }
                    }
                }
            }
            // Startup entries pointing into the install folder
            for rp in RUN_PATHS {
                let rk = RegKey::new(hive, *rp, view);
                for (name, data) in reg.values(&rk) {
                    let Some(data) = data.as_str() else { continue };
                    if points_into(data, &info.location)
                        || classify(&normalize(&name), info, false) == Some(Confidence::High)
                    {
                        out.push(Leftover::reg_value(rk.clone(), &name, "startup entry"));
                    }
                }
            }
        }
    }
}

fn scan_services(reg: &dyn Registry, info: &Info, known: &[&str], out: &mut Vec<Leftover>) {
    let base = RegKey::new(Hive::Hklm, SERVICES_PATH, 64);
    for svc in reg.subkeys(&base) {
        if known.iter().any(|k| k.eq_ignore_ascii_case(&svc)) {
            out.push(Leftover::new(Kind::Service, svc, "known service of this app", Confidence::High));
            continue;
        }
        let vals = reg.values(&base.child(&svc));
        if vals.get("ImagePath").and_then(RegValue::as_str).is_some_and(|img| points_into(img, &info.location)) {
            out.push(Leftover::new(Kind::Service, svc, "service runs from the install folder", Confidence::High));
        }
    }
}

fn read_task_xml(path: &Path) -> String {
    let Ok(raw) = fs::read(path) else { return String::new() };
    let utf16 = |le: bool| {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match raw.get(..2) {
        Some([0xFF, 0xFE]) => utf16(true),
        Some([0xFE, 0xFF]) => utf16(false),
        _ => String::from_utf8_lossy(&raw).into_owned(),
    }
}

fn walk_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        match e.file_type() {
            Ok(t) if t.is_dir() => walk_files(&e.path(), out),
            Ok(t) if t.is_file() => out.push(e.path()),
            _ => {}
        }
    }
}

fn scan_tasks(env: &Environment, info: &Info, out: &mut Vec<Leftover>) {
    if env.tasks_dir.is_empty() || !is_dir(&env.tasks_dir) {
        return;
    }
    let root = Path::new(&env.tasks_dir);
    let mut files = Vec::new();
    walk_files(root, &mut files);
    files.sort();
    for f in files {
        let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace(MAIN_SEPARATOR, "\\");
        let task = format!("\\{rel}");
        let fname = f.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match classify(&normalize(&fname), info, false) {
            Some(Confidence::High) => {
                out.push(Leftover::new(Kind::Task, task, "task name matches app", Confidence::High))
            }
            other => {
                if points_into(&read_task_xml(&f), &info.location) {
                    out.push(Leftover::new(
                        Kind::Task,
                        task,
                        "scheduled task runs from the install folder",
                        Confidence::High,
                    ));
                } else if other == Some(Confidence::Medium) {
                    out.push(Leftover::new(Kind::Task, task, "task name resembles app", Confidence::Medium));
                }
            }
        }
    }
}

pub fn scan_leftovers(program: &Program, env: &Environment, reg: &dyn Registry) -> Vec<Leftover> {
    let own = own_install_folder(program, env, reg);
    let info = info(program, own.clone(), reg);
    let profile = find_profile(program);
    let mut out = Vec::new();

    if reg.exists(&program.key) {
        out.push(Leftover::reg_key(program.key.clone(), "Apps & Features entry", Confidence::High));
    }
    if let Some(loc) = own.filter(|l| Path::new(l).exists()) {
        out.push(fs_item(loc, Confidence::High, "install folder"));
    }
    if let Some(p) = profile {
        for raw in p.paths {
            let path = expand_vars(raw, &env.envvars);
            if !path.contains('%') && Path::new(&path).exists() {
                out.push(fs_item(path, Confidence::High, format!("known {} location", p.name)));
            }
        }
    }
    scan_fs(env, &info, &mut out);
    scan_registry(reg, &info, &mut out);
    scan_services(reg, &info, profile.map_or(&[], |p| p.services), &mut out);
    scan_tasks(env, &info, &mut out);
    let others: Vec<String> = list_programs(reg)
        .into_iter()
        .filter(|o| o.key != program.key)
        .map(|o| o.install_location.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    finalize(out, env, &others)
}

/// `others`: install folders of the other installed programs; a folder holding
/// one of them is never removed.
fn finalize(items: Vec<Leftover>, env: &Environment, others: &[String]) -> Vec<Leftover> {
    let mut seen = HashSet::new();
    let unique: Vec<Leftover> = items
        .into_iter()
        .filter(|it| match it.kind {
            Kind::File | Kind::Dir => {
                env.may_touch(&it.target) && !others.iter().any(|o| safety::is_within(o, &it.target))
            }
            Kind::RegKey => !it.key.as_ref().is_some_and(safety::is_protected_key),
            _ => true,
        })
        .filter(|it| seen.insert((it.kind, it.label().to_lowercase())))
        .collect();
    // Drop paths nested inside a folder that is already listed.
    let dirs: Vec<String> = unique.iter().filter(|i| i.kind == Kind::Dir).map(|i| i.target.clone()).collect();
    let mut out: Vec<Leftover> = unique
        .into_iter()
        .filter(|it| {
            !matches!(it.kind, Kind::File | Kind::Dir)
                || !dirs.iter().any(|d| safety::norm(d) != safety::norm(&it.target) && safety::is_within(&it.target, d))
        })
        .collect();
    out.sort_by_key(|i| (i.confidence != Confidence::High, i.kind.order(), i.label().to_lowercase()));
    out
}
