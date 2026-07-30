//! `WireTerm` foreground application entry point.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    wireterm::app::run()
}
