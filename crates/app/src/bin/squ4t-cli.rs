//! `squ4t-cli`: the ledger from a terminal, like `python3 store.py` and `omarchy-shell b1k.squats`.
//!
//!     squ4t-cli status | tick | add N | undo | pause | resume | snooze | settings INTERVAL SUGGESTED
//!     squ4t-cli open | refresh | test-alert | quit
//!     squ4t-cli autostart on | off | status
//!     squ4t-cli update [install]
//!     [--data-dir DIR]
//!
//! Ledger actions print store.py's JSON. A change made here is passed to the running tray app.

use std::process::ExitCode;

use squ4t::store::{default_dir, Store};
use squ4t::{tell_instance, MSG_OPEN, MSG_QUIT, MSG_REFRESH, MSG_TEST_ALERT};
use squ4t_core::ledger::{ACTIONS, REFUSED_MESSAGE};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Never start a new ledger while the old one could not be brought over: it would hide it.
    if let Err(e) = squ4t::store::migrate_legacy_ledger() {
        println!("{REFUSED_MESSAGE}");
        eprintln!("could not bring the ledger over from {}: {e}", squ4t::store::legacy_dir().display());
        return ExitCode::from(1);
    }
    let mut dir = default_dir();
    if let Some(i) = args.iter().position(|a| a == "--data-dir") {
        if i + 1 >= args.len() {
            eprintln!("--data-dir needs a folder");
            return ExitCode::from(2);
        }
        dir = args.remove(i + 1).into();
        args.remove(i);
    }
    let Some(action) = args.first().cloned() else {
        eprintln!("usage: squ4t-cli status | add N | undo | pause | resume | snooze | settings INTERVAL SUGGESTED | open | test-alert | autostart on|off | update [install]");
        return ExitCode::from(2);
    };
    let values = &args[1..];
    let message = match action.as_str() {
        "open" => Some(MSG_OPEN),
        "refresh" => Some(MSG_REFRESH),
        "test-alert" => Some(MSG_TEST_ALERT),
        "quit" => Some(MSG_QUIT),
        _ => None,
    };
    if let Some(message) = message {
        if tell_instance(&dir, message) {
            return ExitCode::SUCCESS;
        }
        eprintln!("squ4t is not running");
        return ExitCode::from(1);
    }
    if action == "update" {
        return update(values.first().map(String::as_str));
    }
    if action == "autostart" {
        return autostart(values.first().map(String::as_str));
    }
    if !ACTIONS.contains(&action.as_str()) {
        eprintln!("unknown action {action:?}");
        return ExitCode::from(2);
    }
    match Store::new(dir.clone()).run(&action, values) {
        Ok(report) => {
            println!("{}", report.to_json());
            if action != "status" && action != "tick" {
                tell_instance(&dir, MSG_REFRESH);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{REFUSED_MESSAGE}");
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn autostart(choice: Option<&str>) -> ExitCode {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("squ4t.exe")));
    let result = match (choice, exe) {
        (Some("on"), Some(exe)) => squ4t::autostart::set(Some(&exe)),
        (Some("off"), _) => squ4t::autostart::set(None),
        (Some("status") | None, _) => {
            println!("{}", if squ4t::autostart::get().is_some() { "on" } else { "off" });
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("usage: squ4t-cli autostart on | off | status");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// `update` looks for a newer release; `update install` also installs it. A running app keeps
/// running the old program until it is restarted.
fn update(choice: Option<&str>) -> ExitCode {
    use squ4t::update;
    let found = match update::check() {
        Ok(found) => found,
        Err(e) => {
            eprintln!("could not check for updates: {e}");
            return ExitCode::from(1);
        }
    };
    let Some(release) = found else {
        println!("{} is up to date", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    };
    println!("{} available", update::version_text(release.version));
    if choice != Some("install") {
        return ExitCode::SUCCESS;
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) else { return ExitCode::from(1) };
    match update::install(&release, &dir) {
        Ok(()) => {
            println!("updated to {}", update::version_text(release.version));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("could not update: {e}");
            ExitCode::from(1)
        }
    }
}
