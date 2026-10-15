//! NFCoffee Manager — portable TUI client for the NFC_SD_WIFI coffee reader.

mod app;
mod config;
mod db;
mod net;
mod protocol;
mod ui;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::stdout;
use std::path::PathBuf;

fn main() -> Result<()> {
    let dir = resolve_data_dir()?;
    let cfg = config::Config::load(&dir)?;
    let db = db::Db::open(&dir.join("nfcoffee.db"))?;
    let mut app = app::App::new(cfg, db, dir);

    install_panic_hook();

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(out);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let result = app.run(&mut terminal);

    // Always restore the terminal.
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    if let Err(e) = result {
        eprintln!("Application error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

fn resolve_data_dir() -> Result<PathBuf> {
    let preferred = app::data_dir()?;
    match std::fs::create_dir_all(&preferred) {
        Ok(()) => Ok(preferred),
        Err(e) => {
            // Not writable (e.g. a read-only location) -> fall back to CWD.
            let fallback = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("NFCoffeeData");
            std::fs::create_dir_all(&fallback).with_context(|| {
                format!(
                    "creating {} (and fallback {}): {e}",
                    preferred.display(),
                    fallback.display()
                )
            })?;
            eprintln!(
                "note: {} is not writable; using {}",
                preferred.display(),
                fallback.display()
            );
            Ok(fallback)
        }
    }
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_raw_mode().ok();
        let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
