use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    Hklm,
    Hkcu,
}

impl Hive {
    pub fn as_str(self) -> &'static str {
        match self {
            Hive::Hklm => "HKLM",
            Hive::Hkcu => "HKCU",
        }
    }
}

/// A registry key. `view` is 64 or 32 (the WOW64 registry view).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegKey {
    pub hive: Hive,
    pub path: String,
    pub view: u32,
}

impl RegKey {
    pub fn new(hive: Hive, path: impl Into<String>, view: u32) -> Self {
        RegKey { hive, path: path.into(), view }
    }

    pub fn child(&self, name: &str) -> Self {
        RegKey::new(self.hive, format!("{}\\{}", self.path, name), self.view)
    }

    pub fn name(&self) -> &str {
        self.path.rsplit('\\').next().unwrap_or(&self.path)
    }
}

impl fmt::Display for RegKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\\{}", self.hive.as_str(), self.path)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RegValue {
    Str(String),
    Dword(u32),
    Other,
}

impl RegValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RegValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            RegValue::Dword(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub name: String,
    pub key: RegKey,
    pub publisher: String,
    pub version: String,
    pub install_location: String,
    pub uninstall_string: String,
    pub quiet_uninstall_string: String,
    pub size_kb: u32,
    pub install_date: String,
    pub windows_installer: bool,
}

impl Program {
    pub fn new(name: impl Into<String>, key: RegKey) -> Self {
        Program {
            name: name.into(),
            key,
            publisher: String::new(),
            version: String::new(),
            install_location: String::new(),
            uninstall_string: String::new(),
            quiet_uninstall_string: String::new(),
            size_kb: 0,
            install_date: String::new(),
            windows_installer: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Service,
    Task,
    RegValue,
    RegKey,
    Dir,
    File,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Service => "service",
            Kind::Task => "task",
            Kind::RegValue => "reg value",
            Kind::RegKey => "reg key",
            Kind::Dir => "folder",
            Kind::File => "file",
        }
    }

    /// Order in which leftovers are shown and removed: stop services first, files last.
    pub fn order(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Leftover {
    pub kind: Kind,
    /// Path, registry key path, service name or task name.
    pub target: String,
    pub reason: String,
    pub confidence: Confidence,
    pub key: Option<RegKey>,
    pub value_name: String,
}

impl Leftover {
    pub fn new(kind: Kind, target: impl Into<String>, reason: impl Into<String>, confidence: Confidence) -> Self {
        Leftover {
            kind,
            target: target.into(),
            reason: reason.into(),
            confidence,
            key: None,
            value_name: String::new(),
        }
    }

    pub fn reg_key(key: RegKey, reason: impl Into<String>, confidence: Confidence) -> Self {
        Leftover { key: Some(key.clone()), ..Leftover::new(Kind::RegKey, key.path, reason, confidence) }
    }

    pub fn reg_value(key: RegKey, value: &str, reason: impl Into<String>) -> Self {
        Leftover {
            key: Some(key.clone()),
            value_name: value.to_string(),
            ..Leftover::new(Kind::RegValue, key.path, reason, Confidence::High)
        }
    }

    pub fn label(&self) -> String {
        match (&self.kind, &self.key) {
            (Kind::RegKey, Some(k)) => k.to_string(),
            (Kind::RegValue, Some(k)) => format!("{k} [{}]", self.value_name),
            _ => self.target.clone(),
        }
    }
}
