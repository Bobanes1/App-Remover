// No console window for the GUI in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod demo;
mod gui;

use std::process::ExitCode;
use std::sync::Arc;

use app_remover::engine::{backup_base, default_choice, remove_program, Options};
use app_remover::model::Program;
use app_remover::registry::{list_programs, Registry};
use app_remover::scanner::Environment;
use app_remover::system::{NoopSystem, RealSystem, System};

const HELP: &str = "\
App Remover - fast program uninstaller with leftover cleanup

USAGE:
  AppRemover                         open the window
  AppRemover list [FILTER]           list installed programs
  AppRemover remove NAME [OPTIONS]   uninstall NAME and clean up its leftovers

OPTIONS for remove:
  --force           don't run the program's own uninstaller
  --interactive     don't try to make the uninstaller silent
  --include-medium  also delete lower-confidence leftovers
  --dry-run         only show what would be done

  --demo            use made-up programs; never changes anything
";

/// Everything the app needs to talk to the machine.
pub struct Backend {
    pub reg: Arc<dyn Registry>,
    pub sys: Arc<dyn System>,
    pub env: Environment,
    pub demo: bool,
}

fn backend(demo: bool) -> Option<Backend> {
    if demo {
        return Some(Backend {
            reg: Arc::new(demo::registry()),
            sys: Arc::new(NoopSystem),
            env: Environment::default(),
            demo: true,
        });
    }
    #[cfg(windows)]
    {
        Some(Backend {
            reg: Arc::new(app_remover::registry::WinRegistry),
            sys: Arc::new(RealSystem),
            env: Environment::from_os(),
            demo: false,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = RealSystem;
        None
    }
}

/// A GUI-subsystem exe has no console; borrow the parent's one for CLI output.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call without pointers; failure (no parent console) is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

fn pick<'a>(programs: &'a [Program], query: &str) -> Option<&'a Program> {
    let q = query.to_lowercase();
    let exact: Vec<_> = programs.iter().filter(|p| p.name.to_lowercase() == q).collect();
    if exact.len() == 1 {
        return Some(exact[0]);
    }
    let hits: Vec<_> = programs.iter().filter(|p| p.name.to_lowercase().contains(&q)).collect();
    match hits.len() {
        1 => Some(hits[0]),
        0 => {
            println!("No installed program matches '{query}'.");
            None
        }
        _ => {
            println!("'{query}' is ambiguous, be more specific:");
            for p in hits {
                println!("  {} {}", p.name, p.version);
            }
            None
        }
    }
}

fn cli(args: &[String], b: Backend) -> ExitCode {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let positional: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    let programs = list_programs(b.reg.as_ref());
    match args[0].as_str() {
        "list" => {
            let filter = positional.first().map(|s| s.to_lowercase()).unwrap_or_default();
            for p in programs.iter().filter(|p| p.name.to_lowercase().contains(&filter)) {
                println!("{}\t{}\t{}", p.name, p.version, p.publisher);
            }
            ExitCode::SUCCESS
        }
        "remove" => {
            let Some(name) = positional.first() else {
                eprintln!("remove: missing program name\n\n{HELP}");
                return ExitCode::from(2);
            };
            let Some(prog) = pick(&programs, name) else { return ExitCode::FAILURE };
            let opts = Options {
                silent: !flag("--interactive"),
                force: flag("--force"),
                include_medium: flag("--include-medium"),
                dry_run: flag("--dry-run") || b.demo,
                ..Options::default()
            };
            let medium = opts.include_medium;
            let report = remove_program(
                prog,
                &b.env,
                b.reg.as_ref(),
                b.sys.as_ref(),
                &opts,
                &|m| println!("{m}"),
                &|items| default_choice(items, medium),
                &backup_base(),
            );
            if report.results.iter().any(|r| !r.ok) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        other => {
            eprintln!("Unknown command '{other}'.\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let demo = args.iter().any(|a| a == "--demo");
    args.retain(|a| a != "--demo");

    let gui_mode = matches!(args.first().map(String::as_str), None | Some("gui"));
    if !gui_mode {
        attach_console();
        match args[0].as_str() {
            "-h" | "--help" | "help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("App Remover {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }
    let Some(b) = backend(demo) else {
        eprintln!("App Remover only works on Windows. Use --demo to try the interface.");
        return ExitCode::from(2);
    };
    if gui_mode {
        match gui::run(b) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("could not open the window: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        cli(&args, b)
    }
}
