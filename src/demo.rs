//! `--demo`: a fake set of installed programs so the UI can be tried safely
//! (and on non-Windows machines). Nothing is ever changed in this mode.
use app_remover::model::{Hive, RegValue};
use app_remover::registry::{FakeRegistry, UNINSTALL_PATH};

pub fn registry() -> FakeRegistry {
    let reg = FakeRegistry::default();
    let s = |v: &str| RegValue::Str(v.into());
    let apps: &[(&str, &str, &str, u32, &str, &str)] = &[
        (
            "Adobe Creative Cloud",
            "Adobe Inc.",
            "6.4.0.361",
            512_000,
            r"C:\Program Files (x86)\Adobe\Adobe Creative Cloud",
            r#""C:\Program Files (x86)\Adobe\Adobe Creative Cloud\Utils\Creative Cloud Uninstaller.exe""#,
        ),
        (
            "Adobe Photoshop 2024",
            "Adobe Inc.",
            "25.12",
            4_200_000,
            r"C:\Program Files\Adobe\Adobe Photoshop 2024",
            r#""C:\Program Files (x86)\Common Files\Adobe\Adobe Desktop Common\HDBox\Uninstaller.exe" --uninstall=1"#,
        ),
        (
            "7-Zip 24.08 (x64)",
            "Igor Pavlov",
            "24.08",
            5_800,
            r"C:\Program Files\7-Zip\",
            r#""C:\Program Files\7-Zip\Uninstall.exe""#,
        ),
        (
            "Mozilla Firefox (x64 en-US)",
            "Mozilla",
            "131.0",
            245_000,
            r"C:\Program Files\Mozilla Firefox",
            r#""C:\Program Files\Mozilla Firefox\uninstall\helper.exe""#,
        ),
        (
            "Notepad++ (64-bit x64)",
            "Notepad++ Team",
            "8.7",
            14_000,
            r"C:\Program Files\Notepad++",
            r#""C:\Program Files\Notepad++\uninstall.exe""#,
        ),
        (
            "VLC media player",
            "VideoLAN",
            "3.0.21",
            160_000,
            r"C:\Program Files\VideoLAN\VLC",
            r#""C:\Program Files\VideoLAN\VLC\uninstall.exe""#,
        ),
        (
            "Zoom Workplace",
            "Zoom Communications, Inc.",
            "6.2.5",
            380_000,
            "",
            "MsiExec.exe /X{3A8F2E5C-1B6D-4F7A-9C2E-8D4B6A1F3E7C}",
        ),
    ];
    for (name, publisher, version, size, loc, un) in apps {
        reg.add(
            Hive::Hklm,
            &format!("{UNINSTALL_PATH}\\{name}"),
            64,
            &[
                ("DisplayName", s(name)),
                ("Publisher", s(publisher)),
                ("DisplayVersion", s(version)),
                ("EstimatedSize", RegValue::Dword(*size)),
                ("InstallLocation", s(loc)),
                ("UninstallString", s(un)),
            ],
        );
    }
    reg.add(Hive::Hklm, r"Software\Adobe\Adobe Creative Cloud", 32, &[]);
    reg.add(Hive::Hkcu, r"Software\Adobe\Adobe Creative Cloud", 64, &[]);
    reg.add(Hive::Hklm, r"Software\Adobe\Adobe Creative Cloud Experience", 32, &[]);
    reg.add(Hive::Hklm, r"SYSTEM\CurrentControlSet\Services\AdobeUpdateService", 64, &[]);
    reg.add(
        Hive::Hkcu,
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        64,
        &[(
            "Adobe Creative Cloud",
            s(r#""C:\Program Files (x86)\Adobe\Adobe Creative Cloud\ACC\Creative Cloud.exe" --showwindow=false"#),
        )],
    );
    reg
}
