//! omatach — a terminal OBD-II dashboard for Omarchy (OBDLink adapters).
//!
//!   omatach                 launch (adapter picker)
//!   omatach --mock demo     auto-select the demo adapter (or 11bit / 29bit)
//!   omatach --dev           also list the raw mock fixtures in the picker
//!   omatach --vault         list vault entries and exit
//!   omatach --no-log        keep stderr on the terminal (debug)
//!   omatach --version
mod app;
mod catalog;
mod dash;
mod dtc;
mod engine;
mod import;
mod logsink;
mod mock;
mod platform;
mod recorder;
mod sim;
mod support;
mod ui;
mod units;
mod vault;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{Action, App};
use engine::Engine;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("omatach {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--vault") {
        for n in vault::names_list() {
            println!("{n:<20} {} B", vault::bytes(n).len());
        }
        return;
    }
    let auto_mock = args.iter().position(|a| a == "--mock").and_then(|i| args.get(i + 1).cloned());
    let no_log = args.iter().any(|a| a == "--no-log");
    let dev_mocks = args.iter().any(|a| a == "--dev") || auto_mock.as_deref().map_or(false, |m| m != "demo");

    let dirs = logsink::dirs();
    if let Some(d) = &dirs {
        let _ = std::fs::create_dir_all(&d.config);
    }
    let mut app = App::new(dirs.as_ref().map(|d| d.config.as_path()));
    app.recordings_dir = dirs.as_ref().map(|d| d.data.join("recordings"));
    app.auto_select = auto_mock;
    if std::env::var("OMATACH_NO_BRAILLE").map(|v| v == "1").unwrap_or(false) {
        app.units.set_braille(false);
    }
    if !no_log {
        if let Some(d) = &dirs {
            if let Ok(p) = logsink::redirect_stderr(&d.state) {
                app.log_path = Some(p.to_string_lossy().into_owned());
            }
        }
    }
    let cache_dir = dirs.as_ref().map(|d| d.cache.clone());
    if let Some(c) = &cache_dir {
        let _ = std::fs::create_dir_all(c);
    }

    let engine = match Engine::start(cache_dir, dev_mocks) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("omatach: {e}");
            println!("omatach: could not start the OBD engine: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = run(&mut app, &engine) {
        let _ = restore_terminal();
        eprintln!("omatach: {e}");
    }
    app.stop_recording_quietly();
    apply(&engine, app.on_shutdown());
    if app.is_connected() {
        // Wait for the link to really close: a BLE adapter left connected
        // stops advertising and is invisible to the next launch.
        engine.disconnect();
        wait_for_disconnect(&engine, Duration::from_secs(3));
    } else {
        // Still scanning for adapters, or mid-connect: end that too.
        engine.cancel();
    }
    engine.shutdown();
}

fn run(app: &mut App, engine: &Engine) -> io::Result<()> {
    // Restore the terminal on panic so a bug never leaves the shell raw.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    apply(engine, app.rescan_pids());
    apply(engine, app.run_command("connect"));
    let mut dirty = true;
    let mut last_draw = Instant::now();
    let mut last_pulse = Instant::now();
    loop {
        // the recording orb pulses on its own clock
        if app.is_recording() && last_pulse.elapsed() >= Duration::from_millis(500) {
            dirty = true;
            last_pulse = Instant::now();
        }
        if dirty && last_draw.elapsed() >= Duration::from_millis(16) {
            terminal.draw(|f| ui::draw(f, app))?;
            dirty = false;
            last_draw = Instant::now();
        }
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    apply(engine, app.on_key(k));
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
        while let Ok(ev) = engine.rx.try_recv() {
            apply(engine, app.on_engine(ev));
            dirty = true;
        }
        if app.should_quit {
            break;
        }
    }
    restore_terminal()
}

/// Drain engine events until the session reports the link closed (bounded).
fn wait_for_disconnect(engine: &Engine, budget: Duration) {
    let deadline = Instant::now() + budget;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match engine.rx.recv_timeout(left) {
            Ok(engine::EngineEvent::Disconnected(_)) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

fn apply(engine: &Engine, actions: Vec<Action>) {
    for a in actions {
        match a {
            Action::Scan => engine.scan(),
            Action::Select(id) => engine.select(&id),
            Action::Cancel => engine.cancel(),
            Action::Disconnect => engine.disconnect(),
            Action::Subscribe(codes) => engine.subscribe(codes),
            Action::Unsubscribe(codes) => engine.unsubscribe(codes),
            Action::Probe(codes) => engine.probe(codes),
            Action::OpenUrl(url) => support::open_in_browser(&url),
            Action::Quit => {}
        }
    }
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
