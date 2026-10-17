//! Building and running the program's own uninstaller.
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;

use crate::model::Program;

static GUID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[0-9A-Fa-f]{8}-(?:[0-9A-Fa-f]{4}-){3}[0-9A-Fa-f]{12}\}").unwrap());
static EXE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)^(.*?\.(?:exe|msi|bat|cmd|com))(?:\s+(.*))?$").unwrap());
static INNO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^unins\d{3}\.exe$").unwrap());

/// Exit codes meaning success / reboot initiated / reboot required.
pub const OK_CODES: &[i32] = &[0, 1641, 3010];

/// Split a Windows command line into (executable, argument string).
pub fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => (rest[..end].to_string(), rest[end + 1..].trim().to_string()),
            None => (rest.to_string(), String::new()),
        };
    }
    if let Some(c) = EXE.captures(cmd) {
        return (c[1].to_string(), c.get(2).map_or("", |m| m.as_str()).trim().to_string());
    }
    match cmd.split_once(char::is_whitespace) {
        Some((a, b)) => (a.to_string(), b.trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn is_msiexec(exe: &str) -> bool {
    matches!(file_name(exe).to_lowercase().as_str(), "msiexec" | "msiexec.exe")
}

/// Returns (command line, note). The command is `None` if nothing is registered.
pub fn build_command(program: &Program, silent: bool) -> (Option<String>, &'static str) {
    if silent && !program.quiet_uninstall_string.is_empty() {
        return (Some(program.quiet_uninstall_string.clone()), "quiet uninstall string");
    }
    let mut cmd = program.uninstall_string.clone();
    if cmd.is_empty() {
        let key = program.key.name();
        if program.windows_installer && GUID.find(key).is_some_and(|m| m.as_str() == key) {
            cmd = format!("msiexec.exe /x {key}");
        } else {
            return (None, "no uninstall command registered");
        }
    }
    let (exe, args) = split_command(&cmd);
    if is_msiexec(&exe) {
        return match GUID.find(&args) {
            Some(g) => {
                let flags = if silent { " /qn /norestart" } else { "" };
                (Some(format!("msiexec.exe /x {}{flags}", g.as_str())), "Windows Installer")
            }
            None => (Some(cmd), "msiexec (as registered)"),
        };
    }
    if silent && INNO.is_match(file_name(&exe)) {
        return (Some(format!("\"{exe}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART")), "Inno Setup uninstaller");
    }
    (Some(cmd), "registered uninstaller")
}

pub fn uninstaller_available(command: Option<&str>, exists: impl Fn(&str) -> bool) -> bool {
    let Some(command) = command else { return false };
    let (exe, _) = split_command(command);
    is_msiexec(&exe) || exists(&exe)
}

/// Run an uninstall command line and wait for it. Returns (success, message).
pub fn run_command(command: &str, timeout: Duration) -> (bool, String) {
    let (exe, args) = split_command(command);
    let mut cmd = std::process::Command::new(&exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(&args);
    }
    #[cfg(not(windows))]
    cmd.args(args.split_whitespace());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("could not start uninstaller: {e}")),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                return if OK_CODES.contains(&code) {
                    (true, format!("uninstaller finished (code {code})"))
                } else {
                    (false, format!("uninstaller exited with code {code}"))
                };
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(250)),
            Ok(None) => return (false, format!("uninstaller still running after {}s, moving on", timeout.as_secs())),
            Err(e) => return (false, format!("could not wait for uninstaller: {e}")),
        }
    }
}

/// Some uninstallers relaunch themselves from %TEMP% and return at once; wait for
/// the Uninstall registry entry to disappear. True if it did.
pub fn wait_until_gone(still_there: impl Fn() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !still_there() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    !still_there()
}
