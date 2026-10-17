//! Extra knowledge for apps known to scatter themselves around the system.
use regex::Regex;

use crate::model::Program;

pub struct Profile {
    pub name: &'static str,
    /// Case-insensitive regex matched against the display name.
    pub matches: &'static str,
    pub processes: &'static [&'static str],
    pub services: &'static [&'static str],
    /// May contain %VARS%.
    pub paths: &'static [&'static str],
}

pub const PROFILES: &[Profile] = &[Profile {
    name: "Adobe Creative Cloud",
    matches: r"^adobe creative cloud( desktop)?$",
    processes: &[
        "Creative Cloud.exe",
        "Adobe Desktop Service.exe",
        "CCXProcess.exe",
        "CCLibrary.exe",
        "AdobeIPCBroker.exe",
        "Adobe CEF Helper.exe",
        "Creative Cloud Helper.exe",
        "AdobeUpdateService.exe",
        "armsvc.exe",
        "AGMService.exe",
        "AGSService.exe",
    ],
    services: &[
        "AdobeUpdateService",
        "AdobeARMservice",
        "AGMService",
        "AGSService",
        "Adobe Genuine Monitor Service",
        "Adobe Genuine Software Integrity Service",
    ],
    paths: &[
        r"%ProgramFiles(x86)%\Adobe\Adobe Creative Cloud",
        r"%ProgramFiles%\Adobe\Adobe Creative Cloud",
        r"%ProgramFiles%\Adobe\Adobe Creative Cloud Experience",
        r"%ProgramFiles(x86)%\Adobe\Adobe Creative Cloud Experience",
        r"%ProgramData%\Adobe\Creative Cloud",
        r"%APPDATA%\Adobe\Creative Cloud Libraries",
        r"%LOCALAPPDATA%\Adobe\Creative Cloud Libraries",
        r"%LOCALAPPDATA%\Adobe\CoreSync",
        r"%LOCALAPPDATA%\Adobe\OOBE",
    ],
}];

pub fn find_profile(program: &Program) -> Option<&'static Profile> {
    PROFILES
        .iter()
        .find(|p| Regex::new(&format!("(?i){}", p.matches)).map(|re| re.is_match(program.name.trim())).unwrap_or(false))
}
