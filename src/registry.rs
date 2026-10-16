//! Registry access behind a small trait (faked in tests and demo mode) plus the
//! enumeration of installed programs.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::model::{Hive, Program, RegKey, RegValue};

pub const UNINSTALL_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

pub fn uninstall_roots() -> [RegKey; 3] {
    [
        RegKey::new(Hive::Hklm, UNINSTALL_PATH, 64),
        RegKey::new(Hive::Hklm, UNINSTALL_PATH, 32),
        RegKey::new(Hive::Hkcu, UNINSTALL_PATH, 64),
    ]
}

pub trait Registry: Send + Sync {
    fn subkeys(&self, key: &RegKey) -> Vec<String>;
    fn values(&self, key: &RegKey) -> HashMap<String, RegValue>;
    fn exists(&self, key: &RegKey) -> bool;
}

#[cfg(windows)]
pub struct WinRegistry;

#[cfg(windows)]
impl WinRegistry {
    fn open(key: &RegKey) -> std::io::Result<winreg::RegKey> {
        use winreg::enums::*;
        let hive = match key.hive {
            Hive::Hklm => winreg::RegKey::predef(HKEY_LOCAL_MACHINE),
            Hive::Hkcu => winreg::RegKey::predef(HKEY_CURRENT_USER),
        };
        let flag = if key.view == 64 { KEY_WOW64_64KEY } else { KEY_WOW64_32KEY };
        hive.open_subkey_with_flags(&key.path, KEY_READ | flag)
    }
}

#[cfg(windows)]
impl Registry for WinRegistry {
    fn subkeys(&self, key: &RegKey) -> Vec<String> {
        match Self::open(key) {
            Ok(k) => k.enum_keys().filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn values(&self, key: &RegKey) -> HashMap<String, RegValue> {
        use winreg::enums::RegType::*;
        use winreg::types::FromRegValue;
        let mut out = HashMap::new();
        if let Ok(k) = Self::open(key) {
            for (name, v) in k.enum_values().filter_map(Result::ok) {
                let val = match v.vtype {
                    REG_SZ | REG_EXPAND_SZ => String::from_reg_value(&v).map(RegValue::Str).unwrap_or(RegValue::Other),
                    REG_DWORD => u32::from_reg_value(&v).map(RegValue::Dword).unwrap_or(RegValue::Other),
                    _ => RegValue::Other,
                };
                out.insert(name, val);
            }
        }
        out
    }

    fn exists(&self, key: &RegKey) -> bool {
        Self::open(key).is_ok()
    }
}

/// In-memory registry for tests and `--demo` mode.
#[derive(Default)]
pub struct FakeRegistry {
    keys: Mutex<HashMap<RegKey, HashMap<String, RegValue>>>,
}

impl FakeRegistry {
    pub fn add(&self, hive: Hive, path: &str, view: u32, values: &[(&str, RegValue)]) {
        let mut keys = self.keys.lock().unwrap();
        let parts: Vec<&str> = path.split('\\').collect();
        for i in 1..=parts.len() {
            keys.entry(RegKey::new(hive, parts[..i].join("\\"), view)).or_default();
        }
        let entry = keys.get_mut(&RegKey::new(hive, path, view)).unwrap();
        for (n, v) in values {
            entry.insert(n.to_string(), v.clone());
        }
    }

    pub fn delete_key(&self, key: &RegKey) {
        let prefix = format!("{}\\", key.path);
        self.keys.lock().unwrap().retain(|k, _| {
            !(k.hive == key.hive && k.view == key.view && (k.path == key.path || k.path.starts_with(&prefix)))
        });
    }

    pub fn delete_value(&self, key: &RegKey, name: &str) {
        if let Some(v) = self.keys.lock().unwrap().get_mut(key) {
            v.remove(name);
        }
    }
}

impl Registry for FakeRegistry {
    fn subkeys(&self, key: &RegKey) -> Vec<String> {
        let prefix = format!("{}\\", key.path);
        let mut out: Vec<String> = self
            .keys
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.hive == key.hive && k.view == key.view)
            .filter_map(|k| k.path.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('\\'))
            .map(str::to_string)
            .collect();
        out.sort();
        out
    }

    fn values(&self, key: &RegKey) -> HashMap<String, RegValue> {
        self.keys.lock().unwrap().get(key).cloned().unwrap_or_default()
    }

    fn exists(&self, key: &RegKey) -> bool {
        self.keys.lock().unwrap().contains_key(key)
    }
}

const SKIP_RELEASE: &[&str] = &["update", "hotfix", "security update", "update rollup", "service pack"];

fn text(v: &HashMap<String, RegValue>, name: &str) -> String {
    v.get(name).and_then(RegValue::as_str).unwrap_or("").trim().to_string()
}

/// All programs listed in "Apps & Features", minus system components and updates.
pub fn list_programs(reg: &dyn Registry) -> Vec<Program> {
    let mut programs = Vec::new();
    let mut seen = HashSet::new();
    for root in uninstall_roots() {
        for sub in reg.subkeys(&root) {
            let key = root.child(&sub);
            let v = reg.values(&key);
            let name = text(&v, "DisplayName");
            if name.is_empty()
                || v.get("SystemComponent").and_then(RegValue::as_u32) == Some(1)
                || !text(&v, "ParentKeyName").is_empty()
                || SKIP_RELEASE.contains(&text(&v, "ReleaseType").to_lowercase().as_str())
                || (name.starts_with("KB") && name[2..].chars().take(6).all(|c| c.is_ascii_digit()) && name.len() >= 8)
            {
                continue;
            }
            let mut p = Program::new(name, key);
            p.publisher = text(&v, "Publisher");
            p.version = text(&v, "DisplayVersion");
            p.install_location = text(&v, "InstallLocation").trim_matches('"').to_string();
            p.uninstall_string = text(&v, "UninstallString");
            p.quiet_uninstall_string = text(&v, "QuietUninstallString");
            p.size_kb = v.get("EstimatedSize").and_then(RegValue::as_u32).unwrap_or(0);
            p.install_date = text(&v, "InstallDate");
            p.windows_installer = v.get("WindowsInstaller").and_then(RegValue::as_u32) == Some(1);
            let ident = (p.name.to_lowercase(), p.version.clone(), p.uninstall_string.to_lowercase());
            if seen.insert(ident) {
                programs.push(p);
            }
        }
    }
    programs.sort_by_key(|p| p.name.to_lowercase());
    programs
}
