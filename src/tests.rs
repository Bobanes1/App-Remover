use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::cleaner;
use crate::engine::{default_choice, remove_program, Options};
use crate::model::{Confidence, Hive, Kind, Leftover, Program, RegKey, RegValue};
use crate::names::{expand_vars, normalize, publisher_variants};
use crate::registry::{list_programs, FakeRegistry, Registry, UNINSTALL_PATH};
use crate::safety;
use crate::scanner::{scan_leftovers, Environment};
use crate::system::System;
use crate::uninstall;

fn s(v: &str) -> RegValue {
    RegValue::Str(v.into())
}

/// Records calls and applies registry deletions to the fake registry;
/// file deletion uses the real (default) implementation.
struct RecordingSystem {
    reg: Arc<FakeRegistry>,
    calls: Mutex<Vec<String>>,
}

impl RecordingSystem {
    fn new(reg: Arc<FakeRegistry>) -> Self {
        RecordingSystem { reg, calls: Mutex::new(Vec::new()) }
    }
    fn rec(&self, c: String) {
        self.calls.lock().unwrap().push(c);
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl System for RecordingSystem {
    fn kill_processes_in(&self, d: &str) {
        self.rec(format!("kill_in {d}"));
    }
    fn kill_process_names(&self, n: &[&str]) {
        self.rec(format!("kill {}", n.len()));
    }
    fn delete_service(&self, n: &str) -> Result<(), String> {
        self.rec(format!("svc {n}"));
        Ok(())
    }
    fn delete_task(&self, n: &str) -> Result<(), String> {
        self.rec(format!("task {n}"));
        Ok(())
    }
    fn reg_export(&self, key: &RegKey, file: &Path) -> Result<(), String> {
        self.rec(format!("export {key}"));
        fs::write(file, "x").map_err(|e| e.to_string())
    }
    fn reg_delete_key(&self, key: &RegKey) -> Result<(), String> {
        self.rec(format!("delkey {key}"));
        self.reg.delete_key(key);
        Ok(())
    }
    fn reg_delete_value(&self, key: &RegKey, name: &str) -> Result<(), String> {
        self.rec(format!("delval {key} {name}"));
        self.reg.delete_value(key, name);
        Ok(())
    }
}

#[test]
fn names() {
    assert_eq!(normalize("7-Zip 23.01 (x64)"), "7zip");
    assert_eq!(normalize("Adobe Creative Cloud"), "adobecreativecloud");
    let v = publisher_variants("Adobe Systems Incorporated");
    assert!(v.contains("adobe") && v.contains("adobesystems"));
    let env = HashMap::from([("programfiles(x86)".to_string(), "C:/P".to_string())]);
    assert_eq!(expand_vars(r"%ProgramFiles(x86)%\A", &env), "C:/P\\A");
    assert_eq!(expand_vars("%NOPE%\\x", &env), "%NOPE%\\x");
}

fn prog(uninstall: &str, quiet: &str) -> Program {
    let mut p = Program::new("X", RegKey::new(Hive::Hklm, format!("{UNINSTALL_PATH}\\X"), 64));
    p.uninstall_string = uninstall.into();
    p.quiet_uninstall_string = quiet.into();
    p
}

#[test]
fn split_command() {
    assert_eq!(uninstall::split_command(r#""C:\a b\u.exe" /S"#), (r"C:\a b\u.exe".into(), "/S".into()));
    assert_eq!(uninstall::split_command(r"C:\a b\u.exe /S"), (r"C:\a b\u.exe".into(), "/S".into()));
    assert_eq!(uninstall::split_command("tool"), ("tool".into(), "".into()));
}

#[test]
fn msi_becomes_silent_uninstall() {
    let g = "{12345678-1234-1234-1234-123456789ABC}";
    let (cmd, _) = uninstall::build_command(&prog(&format!("MsiExec.exe /I{g}"), ""), true);
    assert_eq!(cmd.unwrap(), format!("msiexec.exe /x {g} /qn /norestart"));
}

#[test]
fn quiet_string_and_inno() {
    let p = prog("u.exe", "u.exe /quiet");
    assert_eq!(uninstall::build_command(&p, true).0.unwrap(), "u.exe /quiet");
    assert_eq!(uninstall::build_command(&p, false).0.unwrap(), "u.exe");
    let inno = prog(r#""C:\A\unins000.exe""#, "");
    assert!(uninstall::build_command(&inno, true).0.unwrap().contains("/VERYSILENT"));
}

#[test]
fn missing_uninstaller_detected() {
    let (cmd, _) = uninstall::build_command(&prog(r#""C:\gone\u.exe" --x"#, ""), true);
    assert!(!uninstall::uninstaller_available(cmd.as_deref(), |_| false));
    assert!(uninstall::build_command(&prog("", ""), true).0.is_none());
}

#[test]
fn protected_paths_and_keys() {
    let root = if cfg!(windows) { r"C:\" } else { "/" };
    let pf = Path::new(root).join("pf").to_string_lossy().into_owned();
    let common = Path::new(&pf).join("Common Files").to_string_lossy().into_owned();
    let app = Path::new(&pf).join("App").to_string_lossy().into_owned();
    let prot = vec![pf.clone(), common.clone()];
    assert!(safety::is_protected_path(&pf, &prot));
    assert!(safety::is_protected_path(root, &prot));
    assert!(safety::is_protected_path(&common, &prot));
    assert!(safety::is_protected_path("relative/dir", &prot));
    assert!(!safety::is_protected_path(&app, &prot));
    assert!(safety::is_protected_key(&RegKey::new(Hive::Hklm, "Software", 64)));
    assert!(safety::is_protected_key(&RegKey::new(Hive::Hklm, r"Software\Microsoft", 64)));
    assert!(!safety::is_protected_key(&RegKey::new(Hive::Hklm, r"Software\Adobe\Adobe Creative Cloud", 64)));
}

/// Adobe Creative Cloud whose uninstaller is missing, next to Photoshop which must survive.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: String,
    pf86: String,
    loc: String,
    photoshop: String,
    lnk: String,
    pd_cc: String,
    reg: Arc<FakeRegistry>,
    env: Environment,
    ukey: RegKey,
}

fn p(base: &str, parts: &[&str]) -> String {
    let mut pb = Path::new(base).to_path_buf();
    pb.extend(parts);
    pb.to_string_lossy().into_owned()
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_string_lossy().into_owned();
    let pf86 = p(&root, &["Program Files (x86)"]);
    let pd = p(&root, &["ProgramData"]);
    let local = p(&root, &["Local"]);
    let menu = p(&root, &["StartMenu"]);
    let loc = p(&pf86, &["Adobe", "Adobe Creative Cloud"]);
    let photoshop = p(&pf86, &["Adobe", "Adobe Photoshop 2024"]);
    let lnk = p(&menu, &["Adobe", "Adobe Creative Cloud.lnk"]);
    let pd_cc = p(&pd, &["Adobe", "Creative Cloud"]);
    for d in [&local, &p(&loc, &["Utils"]), &photoshop, &p(&menu, &["Adobe"]), &pd_cc] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p(&loc, &["x.dll"]), "").unwrap();
    fs::write(&lnk, "").unwrap();
    fs::write(p(&menu, &["Adobe", "Adobe Photoshop 2024.lnk"]), "").unwrap();

    let reg = Arc::new(FakeRegistry::default());
    let upath = format!("{UNINSTALL_PATH}\\Adobe Creative Cloud");
    reg.add(
        Hive::Hklm,
        &upath,
        32,
        &[
            ("DisplayName", s("Adobe Creative Cloud")),
            ("Publisher", s("Adobe Inc.")),
            ("DisplayVersion", s("6.0")),
            ("InstallLocation", s(&loc)),
            ("UninstallString", s(&format!("\"{}\"", p(&loc, &["Utils", "Creative Cloud Uninstaller.exe"])))),
        ],
    );
    reg.add(
        Hive::Hklm,
        &format!("{UNINSTALL_PATH}\\Hidden"),
        64,
        &[("DisplayName", s("Hidden")), ("SystemComponent", RegValue::Dword(1))],
    );
    reg.add(Hive::Hklm, r"Software\Adobe\Adobe Creative Cloud", 32, &[("x", RegValue::Dword(1))]);
    reg.add(Hive::Hklm, r"Software\Adobe\Adobe Photoshop 2024", 32, &[("x", RegValue::Dword(1))]);
    reg.add(Hive::Hklm, r"Software\Microsoft\Foo", 64, &[]);
    reg.add(Hive::Hkcu, r"Software\Adobe\Adobe Creative Cloud", 64, &[("x", RegValue::Dword(1))]);
    reg.add(Hive::Hklm, r"SYSTEM\CurrentControlSet\Services\AdobeUpdateService", 64, &[("ImagePath", s("x"))]);
    reg.add(Hive::Hklm, r"SYSTEM\CurrentControlSet\Services\Other", 64, &[("ImagePath", s(r"C:\Win\o.exe"))]);
    reg.add(
        Hive::Hkcu,
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        64,
        &[("CCStartup", s(&format!("\"{}\" --bg", p(&loc, &["cc.exe"])))), ("Keep", s(r"C:\Other\o.exe"))],
    );
    let env = Environment {
        file_roots: vec![pf86.clone(), pd.clone(), local.clone()],
        shortcut_roots: vec![menu.clone()],
        tasks_dir: String::new(),
        protected: vec![pf86.clone(), pd.clone(), local.clone(), menu.clone(), root.clone()],
        envvars: HashMap::from([
            ("ProgramFiles(x86)".into(), pf86.clone()),
            ("ProgramData".into(), pd.clone()),
            ("LOCALAPPDATA".into(), local.clone()),
        ]),
    };
    Fixture { _tmp: tmp, root, pf86, loc, photoshop, lnk, pd_cc, reg, env, ukey: RegKey::new(Hive::Hklm, upath, 32) }
}

fn cc(f: &Fixture) -> Program {
    list_programs(f.reg.as_ref()).into_iter().find(|p| p.name == "Adobe Creative Cloud").unwrap()
}

#[test]
fn lists_programs_without_system_components() {
    let f = fixture();
    let progs = list_programs(f.reg.as_ref());
    assert_eq!(progs.len(), 1);
    assert_eq!(progs[0].key, f.ukey);
}

#[test]
fn scan_finds_only_this_app() {
    let f = fixture();
    let found = scan_leftovers(&cc(&f), &f.env, f.reg.as_ref());
    let labels: Vec<String> = found.iter().map(Leftover::label).collect();
    for want in [&f.loc, &f.lnk, &f.pd_cc] {
        assert!(labels.contains(want), "missing {want}: {labels:#?}");
    }
    assert!(found.iter().any(|i| i.kind == Kind::Service && i.target == "AdobeUpdateService"));
    assert!(labels.contains(&r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run [CCStartup]".to_string()));
    assert!(!labels.contains(&f.photoshop));
    assert!(!labels.contains(&p(&f.pf86, &["Adobe"])));
    assert!(!labels
        .iter()
        .any(|l| l.contains("Photoshop") || l.contains("Other") || l.contains("Foo") || l.contains("[Keep]")));
    assert!(!labels.iter().any(|l| l.ends_with(r"\Software\Adobe")));
    // nothing nested under the install folder is listed separately
    assert!(!labels.iter().any(|l| l.len() > f.loc.len() && l.starts_with(&f.loc)));
}

#[test]
fn full_removal_without_uninstaller() {
    let f = fixture();
    let sys = RecordingSystem::new(f.reg.clone());
    let logs = Mutex::new(Vec::<String>::new());
    let backup = p(&f.root, &["bk"]);
    let report = remove_program(
        &cc(&f),
        &f.env,
        f.reg.as_ref(),
        &sys,
        &Options::default(),
        &|m| logs.lock().unwrap().push(m.to_string()),
        &|items| default_choice(items, false),
        Path::new(&backup),
    );
    assert!(logs.lock().unwrap().iter().any(|m| m.contains("Uninstaller unavailable")));
    let failed: Vec<_> = report.results.iter().filter(|r| !r.ok).map(|r| (r.item.label(), r.message.clone())).collect();
    assert!(failed.is_empty(), "{failed:?}");
    assert!(!Path::new(&f.loc).exists());
    assert!(!Path::new(&f.lnk).exists());
    assert!(Path::new(&f.photoshop).is_dir(), "sibling app must survive");
    assert!(Path::new(&p(&f.pf86, &["Adobe"])).is_dir());
    assert!(!f.reg.exists(&f.ukey));
    assert!(f.reg.exists(&RegKey::new(Hive::Hklm, r"Software\Adobe\Adobe Photoshop 2024", 32)));
    assert!(f.reg.exists(&RegKey::new(Hive::Hklm, r"Software\Microsoft\Foo", 64)));
    let run = f.reg.values(&RegKey::new(Hive::Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run", 64));
    assert!(run.contains_key("Keep") && !run.contains_key("CCStartup"));
    assert!(sys.calls().contains(&"svc AdobeUpdateService".to_string()));
    // registry backups were written
    let backups: Vec<_> = fs::read_dir(&backup).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    assert!(fs::read_dir(backups[0].path()).unwrap().count() > 0);
}

#[test]
fn empty_publisher_folder_is_pruned() {
    let f = fixture();
    fs::remove_dir_all(&f.photoshop).unwrap();
    let sys = RecordingSystem::new(f.reg.clone());
    remove_program(
        &cc(&f),
        &f.env,
        f.reg.as_ref(),
        &sys,
        &Options::default(),
        &|_| {},
        &|items| default_choice(items, false),
        Path::new(&p(&f.root, &["bk"])),
    );
    assert!(!Path::new(&p(&f.pf86, &["Adobe"])).exists());
    assert!(Path::new(&f.pf86).is_dir());
}

#[test]
fn dry_run_changes_nothing() {
    let f = fixture();
    let sys = RecordingSystem::new(f.reg.clone());
    let opts = Options { dry_run: true, ..Options::default() };
    let r = remove_program(
        &cc(&f),
        &f.env,
        f.reg.as_ref(),
        &sys,
        &opts,
        &|_| {},
        &|items| default_choice(items, false),
        Path::new(&p(&f.root, &["bk"])),
    );
    assert!(!r.found.is_empty());
    assert!(sys.calls().is_empty());
    assert!(Path::new(&f.loc).is_dir());
    assert!(f.reg.exists(&f.ukey));
}

#[test]
fn cleaner_refuses_protected_locations() {
    let f = fixture();
    let sys = RecordingSystem::new(f.reg.clone());
    let items = vec![
        Leftover::new(Kind::Dir, f.pf86.clone(), "", Confidence::High),
        Leftover::reg_key(RegKey::new(Hive::Hklm, "Software", 64), "", Confidence::High),
    ];
    let res = cleaner::clean(&items, &f.env, &sys, Path::new(&f.root), &|_| {});
    assert!(res.iter().all(|r| !r.ok));
    assert!(Path::new(&f.pf86).is_dir());
    assert!(sys.calls().is_empty());
}

#[cfg(unix)]
#[test]
fn delete_does_not_follow_symlinks() {
    let f = fixture();
    let outside = p(&f.root, &["precious"]);
    fs::create_dir_all(&outside).unwrap();
    fs::write(p(&outside, &["keep.txt"]), "x").unwrap();
    std::os::unix::fs::symlink(&outside, p(&f.loc, &["link"])).unwrap();
    crate::system::force_remove(Path::new(&f.loc)).unwrap();
    assert!(!Path::new(&f.loc).exists());
    assert!(Path::new(&p(&outside, &["keep.txt"])).exists());
}

#[test]
fn finds_scheduled_task_in_utf16_xml() {
    let mut f = fixture();
    let tasks = p(&f.root, &["Tasks"]);
    fs::create_dir_all(p(&tasks, &["Vendor"])).unwrap();
    let xml = format!("<Task><Exec><Command>{}</Command></Exec></Task>", p(&f.loc, &["cc.exe"]));
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(p(&tasks, &["Vendor", "Updater"]), bytes).unwrap();
    fs::write(p(&tasks, &["Unrelated"]), "<Task><Command>C:\\Other\\o.exe</Command></Task>").unwrap();
    f.env.tasks_dir = tasks;
    let found = scan_leftovers(&cc(&f), &f.env, f.reg.as_ref());
    let found_tasks: Vec<_> = found.iter().filter(|i| i.kind == Kind::Task).map(|i| i.target.as_str()).collect();
    assert_eq!(found_tasks, ["\\Vendor\\Updater"]);
}

#[test]
fn program_drive_detection() {
    use crate::disk::program_drive;
    let mut a = prog(r#""C:\Windows\SysWOW64\uninst.exe" /x"#, "");
    assert_eq!(program_drive(&a), None, "an uninstaller under Windows says nothing");
    a.display_icon = r#""e:\Tools\app.exe",0"#.into();
    assert_eq!(program_drive(&a).as_deref(), Some("E:"));
    a.install_location = r"D:\Games\Foo".into();
    assert_eq!(program_drive(&a).as_deref(), Some("D:"), "install folder wins");
    let msi = prog("MsiExec.exe /X{12345678-1234-1234-1234-123456789ABC}", "");
    assert_eq!(program_drive(&msi), None);
    let plain = prog(r#""F:\Apps\x\uninstall.exe""#, "");
    assert_eq!(program_drive(&plain).as_deref(), Some("F:"));
}

#[test]
fn folder_size_counts_files_but_not_links() {
    let f = fixture();
    fs::write(p(&f.loc, &["big.bin"]), vec![0u8; 5000]).unwrap();
    fs::write(p(&f.loc, &["Utils", "small.bin"]), vec![0u8; 100]).unwrap();
    // x.dll from the fixture is empty: 5000 + 100 bytes -> 5 KB rounded up
    assert_eq!(crate::disk::folder_size_kb(Path::new(&f.loc)), 5);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&f.photoshop, p(&f.loc, &["link"])).unwrap();
        fs::write(p(&f.photoshop, &["huge.bin"]), vec![0u8; 50_000]).unwrap();
        assert_eq!(crate::disk::folder_size_kb(Path::new(&f.loc)), 5);
    }
}
