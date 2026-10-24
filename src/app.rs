//! App state + input handling. Pure: keys and engine events go in,
//! `Action`s for the engine come out — so the whole shell is testable headless.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use obd_session_rs::platform::ConnectorInfo;
use obd_session_rs::vehicle_info::VehicleInfo;

use crate::catalog::Catalog;
use crate::dash::Dashboard;
use crate::dtc::{self, ClearResult, Dtc, Kind, Library};
use crate::import;
use crate::recorder::{Recorder, Summary};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use crate::engine::{DiscoveryFacts, EngineEvent};
use crate::units::Units;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Connect,
    VehicleInfo,
    TroubleCodes,
    Dashboard,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::Connect => "Connect",
            Screen::VehicleInfo => "Vehicle Info",
            Screen::TroubleCodes => "Trouble Codes",
            Screen::Dashboard => "Dashboard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    Idle,
    Scanning,
    Connecting,
    Connected,
    Disconnecting,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    Help,
    Palette { input: String },
    Picker,
    /// The KOEO confirm before a `04` clear.
    Koeo,
    /// Support / donate.
    Support,
    /// "Recording saved" popup after `R` stops a recording.
    RecordingSaved(Summary),
}

/// Trouble-codes screen state.
pub struct DtcState {
    pub lists: [Option<Vec<Dtc>>; 3],
    pub tab: usize,
    pub cursor: usize,
    /// Reads in flight (`03`/`07`/`0A`/`04`).
    pub pending: Vec<String>,
    pub banner: Option<(String, bool)>,
}

impl DtcState {
    pub fn new() -> Self {
        Self { lists: [None, None, None], tab: 0, cursor: 0, pending: Vec::new(), banner: None }
    }
    pub fn current(&self) -> &[Dtc] {
        self.lists[self.tab].as_deref().unwrap_or(&[])
    }
}

/// `+` picker state.
#[derive(Debug, Clone, Default)]
pub struct PickerState {
    pub input: String,
    pub cursor: usize,
    pub only_supported: bool,
}

#[derive(Debug, Clone)]
pub struct PickerRow {
    pub id: String,
    pub code: String,
    pub name: String,
    pub value: String,
    pub source: String,
    pub supported: bool,
    pub on: bool,
}

/// What the UI asks the engine to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Scan,
    Select(String),
    Cancel,
    Disconnect,
    Quit,
    /// `(code, header)` targets to poll / stop polling.
    Subscribe(Vec<(String, Option<String>)>),
    Unsubscribe(Vec<(String, Option<String>)>),
    /// One-shot commands (`03`/`07`/`0A`/`04`, enable-test probes).
    Probe(Vec<String>),
    /// Open a URL in the browser.
    OpenUrl(String),
}

/// Palette commands: (name, description).
pub const COMMANDS: &[(&str, &str)] = &[
    ("connect", "adapter picker"),
    ("rescan", "scan for adapters again"),
    ("disconnect", "drop the adapter link"),
    ("info", "vehicle info"),
    ("trouble", "trouble codes"),
    ("dashboard", "live gauges"),
    ("pids", "rescan the pids/ folder (Torque CSVs)"),
    ("support", "donate / support omatach"),
    ("help", "key reference"),
    ("quit", "exit omatach"),
];

pub struct App {
    pub screen: Screen,
    pub conn: ConnState,
    pub overlay: Overlay,
    pub connectors: Vec<ConnectorInfo>,
    pub selected: usize,
    /// Humanized last progress step.
    pub status: String,
    pub facts: DiscoveryFacts,
    pub vehicle: Option<Box<VehicleInfo>>,
    pub should_quit: bool,
    /// Vertical scroll of the current data screen (clamped at draw time).
    pub scroll: u16,
    /// `--mock <key>`: select `mock:<key>` as soon as it appears.
    pub auto_select: Option<String>,
    pub log_path: Option<String>,
    pub catalog: Catalog,
    pub dash: Dashboard,
    pub units: Units,
    pub picker: PickerState,
    pub dtc: DtcState,
    pub dtc_lib: Library,
    /// `~/.config/omatach/pids/` — Torque CSV drop folder.
    pub pids_dir: Option<PathBuf>,
    /// Last import summary, shown above the dashboard grid.
    pub pids_banner: Option<String>,
    /// Targets currently subscribed on the engine (diffed by `sync_subscriptions`).
    subscribed: HashSet<(String, Option<String>)>,
    /// `<data>/recordings/` — where `R` writes.
    pub recordings_dir: Option<PathBuf>,
    pub recorder: Option<Recorder>,
    /// Probe actions held from `vehicle_info` until `connected`.
    after_connect: Vec<Action>,
}

impl App {
    pub fn new(config_dir: Option<&std::path::Path>) -> Self {
        Self {
            screen: Screen::Connect,
            conn: ConnState::Idle,
            overlay: Overlay::None,
            connectors: Vec::new(),
            selected: 0,
            status: String::new(),
            facts: DiscoveryFacts::default(),
            vehicle: None,
            should_quit: false,
            scroll: 0,
            auto_select: None,
            log_path: None,
            catalog: Catalog::from_vault(),
            dash: Dashboard::new(config_dir),
            units: Units::load(config_dir),
            picker: PickerState::default(),
            dtc: DtcState::new(),
            dtc_lib: Library::from_vault(),
            pids_dir: config_dir.map(|d| d.join("pids")),
            pids_banner: None,
            subscribed: HashSet::new(),
            recordings_dir: None,
            recorder: None,
            after_connect: Vec::new(),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// `R`: start a recording of every dashboard cell, or stop and show the popup.
    pub fn toggle_recording(&mut self) {
        if let Some(r) = self.recorder.take() {
            let summary = r.stop();
            self.overlay = Overlay::RecordingSaved(summary);
            return;
        }
        let Some(dir) = self.recordings_dir.clone() else { return };
        if !self.is_connected() || self.dash.cells.is_empty() {
            return;
        }
        // "SHORT (unit)" labels, like the mac dashboard's toggleRecording.
        let columns: Vec<(String, String)> = self
            .dash
            .cells
            .iter()
            .filter_map(|c| self.catalog.get(&c.id))
            .map(|e| {
                let short = e.short_name.clone().unwrap_or_else(|| e.code.clone());
                let unit = e.unit.trim();
                let label = if unit.is_empty() || unit == "-" { short } else { format!("{short} ({unit})") };
                (e.id.clone(), label)
            })
            .collect();
        match Recorder::start(&dir, columns) {
            Ok(r) => self.recorder = Some(r),
            Err(e) => eprintln!("recorder: cannot start: {e}"),
        }
    }

    /// Stop without the popup (quit / disconnect).
    pub fn stop_recording_quietly(&mut self) {
        if let Some(r) = self.recorder.take() {
            let _ = r.stop();
        }
    }

    fn record_snapshot(&mut self) {
        let Some(rec) = self.recorder.as_mut() else { return };
        let mut values: HashMap<String, f64> = HashMap::new();
        for c in &self.dash.cells {
            if let Some(v) = self.dash.latest.get(&c.id).and_then(|s| s.value.as_f64()) {
                values.insert(c.id.clone(), v);
            }
        }
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        rec.record(&values, ts);
    }

    /// Scan the pids/ folder and layer its entries over the built-ins; skipped
    /// rows go to the log (stderr → omatach.log). Re-syncs subscriptions.
    pub fn rescan_pids(&mut self) -> Vec<Action> {
        let Some(dir) = self.pids_dir.clone() else { return Vec::new() };
        let _ = std::fs::create_dir_all(&dir);
        let report = import::scan_dir(&dir);
        for s in &report.skipped {
            eprintln!("pids/ import: {s}");
        }
        // Silent when the folder is simply empty — a user who never made it shouldn't see a nag.
        self.pids_banner = if report.files > 0 || !report.skipped.is_empty() { Some(report.summary()) } else { None };
        self.catalog.reload_extras(report.entries);
        self.sync_subscriptions()
    }

    /// The recording indicator for the bottom-right corner: `(pulse_on, text)`.
    /// The orb pulses at ~1 Hz on the recorder's clock (main ticks a redraw while recording).
    pub fn recording_status(&self) -> Option<(bool, String)> {
        self.recorder.as_ref().map(|r| {
            let e = r.elapsed();
            let s = e as u64;
            ((e * 2.0) as u64 % 2 == 0, format!("REC {:02}:{:02} · {} rows", s / 60, s % 60, r.rows))
        })
    }

    /// The bottom bar's right-hand status: sensor probes in flight, then the last pids/ import.
    pub fn footer_status(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.screen == Screen::Dashboard && !self.dash.pending_tests.is_empty() {
            parts.push(format!("checking sensors {}/{}…", self.dash.tests_done(), self.dash.tests_total));
        }
        if self.screen == Screen::Dashboard {
            if let Some(b) = &self.pids_banner {
                parts.push(b.clone());
            }
        }
        if parts.is_empty() { None } else { Some(parts.join(" · ")) }
    }

    /// Diff what the dashboard needs against what is subscribed. Only the
    /// Dashboard polls: on any other screen the need is empty, so leaving it
    /// stops the polling (the bus is quiet for a DTC read or clear) and coming
    /// back resumes it.
    fn sync_subscriptions(&mut self) -> Vec<Action> {
        if !self.is_connected() {
            return Vec::new();
        }
        let need: HashSet<(String, Option<String>)> = if self.screen == Screen::Dashboard {
            self.dash.wire_targets(&self.catalog).into_iter().collect()
        } else {
            HashSet::new()
        };
        let add: Vec<_> = need.difference(&self.subscribed).cloned().collect();
        let drop: Vec<_> = self.subscribed.difference(&need).cloned().collect();
        self.subscribed = need;
        let mut a = Vec::new();
        if !add.is_empty() {
            let mut add = add;
            add.sort();
            a.push(Action::Subscribe(add));
        }
        if !drop.is_empty() {
            let mut drop = drop;
            drop.sort();
            a.push(Action::Unsubscribe(drop));
        }
        a
    }

    fn wmi(&self) -> String {
        self.vehicle.as_ref().map(|v| v.wmi.clone()).unwrap_or_default()
    }

    /// Issue the three DTC reads.
    pub fn read_dtcs(&mut self) -> Vec<Action> {
        if !self.is_connected() {
            return Vec::new();
        }
        let cmds: Vec<String> = Kind::ALL.iter().map(|k| k.command().to_string()).collect();
        self.dtc.pending = cmds.clone();
        vec![Action::Probe(cmds)]
    }

    pub fn is_connected(&self) -> bool {
        self.conn == ConnState::Connected
    }

    // ---- keys -----------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return vec![Action::Quit];
        }
        match self.overlay.clone() {
            Overlay::Help => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            Overlay::Palette { mut input } => match key.code {
                KeyCode::Esc => {
                    self.overlay = Overlay::None;
                    Vec::new()
                }
                KeyCode::Enter => {
                    self.overlay = Overlay::None;
                    match palette_matches(&input).first() {
                        Some(cmd) => self.run_command(cmd),
                        None => Vec::new(),
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    self.overlay = Overlay::Palette { input };
                    Vec::new()
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.overlay = Overlay::Palette { input };
                    Vec::new()
                }
                _ => Vec::new(),
            },
            Overlay::Picker => self.on_picker_key(key),
            Overlay::RecordingSaved(_) => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            Overlay::Support => match key.code {
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Enter => vec![Action::OpenUrl(crate::support::SUPPORT_URL.to_string())],
                _ => {
                    self.overlay = Overlay::None;
                    Vec::new()
                }
            },
            Overlay::Koeo => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.overlay = Overlay::None;
                    self.dtc.pending = vec!["04".into()];
                    self.dtc.banner = None;
                    vec![Action::Probe(vec!["04".into()])]
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
                    self.overlay = Overlay::None;
                    Vec::new()
                }
                _ => Vec::new(),
            },
            Overlay::None => self.on_screen_key(key),
        }
    }

    fn on_screen_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Char('q') => self.run_command("quit"),
            KeyCode::Char('?') => self.run_command("help"),
            KeyCode::Char('/') => {
                self.overlay = Overlay::Palette { input: String::new() };
                Vec::new()
            }
            KeyCode::Char('c') => self.run_command("connect"),
            KeyCode::Char('$') => self.run_command("support"),
            KeyCode::Char('i') => self.run_command("info"),
            KeyCode::Char('t') => self.run_command("trouble"),
            KeyCode::Char('d') => self.run_command("dashboard"),
            _ if self.screen == Screen::Connect => self.on_connect_key(key),
            _ if self.screen == Screen::Dashboard => self.on_dashboard_key(key),
            _ if self.screen == Screen::TroubleCodes => self.on_dtc_key(key),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                Vec::new()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                Vec::new()
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(10);
                Vec::new()
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(10);
                Vec::new()
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.scroll = 0;
                Vec::new()
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.scroll = u16::MAX; // clamped to the last page at draw time
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_connect_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Vec::new()
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.connectors.len() {
                    self.selected += 1;
                }
                Vec::new()
            }
            KeyCode::Enter => self.select_current(),
            KeyCode::Char('r') => self.run_command("rescan"),
            KeyCode::Char('x') => self.run_command("disconnect"),
            KeyCode::Esc if self.conn == ConnState::Connecting => {
                self.status = "Cancelled".into();
                self.conn = ConnState::Scanning;
                vec![Action::Cancel, Action::Scan]
            }
            _ => Vec::new(),
        }
    }

    fn on_dashboard_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.picker = PickerState { input: String::new(), cursor: 0, only_supported: true };
                self.overlay = Overlay::Picker;
                Vec::new()
            }
            KeyCode::Char('-') | KeyCode::Delete | KeyCode::Backspace => {
                if self.dash.remove_selected().is_none() {
                    return Vec::new();
                }
                self.sync_subscriptions()
            }
            KeyCode::Char('r') => self.rescan_pids(),
            KeyCode::Char('R') => {
                self.toggle_recording();
                Vec::new()
            }
            KeyCode::Left | KeyCode::Char('h') if shift || key.code == KeyCode::Char('H') => {
                self.dash.move_selected(-1);
                Vec::new()
            }
            KeyCode::Right | KeyCode::Char('l') if shift || key.code == KeyCode::Char('L') => {
                self.dash.move_selected(1);
                Vec::new()
            }
            KeyCode::Char('H') => {
                self.dash.move_selected(-1);
                Vec::new()
            }
            KeyCode::Char('L') => {
                self.dash.move_selected(1);
                Vec::new()
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.dash.select_prev();
                Vec::new()
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.dash.select_next();
                Vec::new()
            }
            KeyCode::Char('s') => {
                self.dash.cycle_style();
                Vec::new()
            }
            KeyCode::Char('u') => {
                self.units.toggle();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_dtc_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.dtc.tab = (self.dtc.tab + 1) % 3;
                self.dtc.cursor = 0;
                Vec::new()
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.dtc.tab = (self.dtc.tab + 2) % 3;
                self.dtc.cursor = 0;
                Vec::new()
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.dtc.cursor + 1 < self.dtc.current().len() {
                    self.dtc.cursor += 1;
                }
                Vec::new()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dtc.cursor = self.dtc.cursor.saturating_sub(1);
                Vec::new()
            }
            KeyCode::Char('r') => {
                self.dtc.banner = None;
                self.read_dtcs()
            }
            KeyCode::Char('x') if self.is_connected() && self.dtc.pending.is_empty() => {
                self.overlay = Overlay::Koeo;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_picker_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Tab => {
                self.picker.only_supported = !self.picker.only_supported;
                self.picker.cursor = 0;
                Vec::new()
            }
            KeyCode::Up => {
                self.picker.cursor = self.picker.cursor.saturating_sub(1);
                Vec::new()
            }
            KeyCode::Down => {
                let n = self.picker_rows().len();
                if self.picker.cursor + 1 < n {
                    self.picker.cursor += 1;
                }
                Vec::new()
            }
            KeyCode::Char(' ') => {
                let rows = self.picker_rows();
                let Some(row) = rows.get(self.picker.cursor).cloned() else { return Vec::new() };
                if row.on {
                    self.dash.remove(&row.id);
                } else {
                    self.dash.add(&row.id);
                }
                self.sync_subscriptions()
            }
            KeyCode::Backspace => {
                self.picker.input.pop();
                self.picker.cursor = 0;
                Vec::new()
            }
            KeyCode::Char(c) => {
                self.picker.input.push(c);
                self.picker.cursor = 0;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Picker rows: not hidden by an enable test, matching the filter, supported
    /// first (then by code), `only_supported` drops the rest.
    pub fn picker_rows(&self) -> Vec<PickerRow> {
        let needle = self.picker.input.trim().to_lowercase();
        let mut rows: Vec<PickerRow> = self
            .catalog
            .entries
            .iter()
            .filter(|e| !self.dash.hidden.contains(&e.id))
            .filter(|e| {
                needle.is_empty()
                    || e.name.to_lowercase().contains(&needle)
                    || e.code.to_lowercase().contains(&needle)
                    || e.short_name.as_deref().map_or(false, |s| s.to_lowercase().contains(&needle))
            })
            .map(|e| {
                let supported = if e.computed {
                    let c = self.catalog.closure(&e.id);
                    c.missing.is_empty() && !c.cycle && c.inputs.iter().all(|(code, _)| !code.starts_with("01") || self.dash.supported.contains(code))
                } else if e.code.starts_with("01") {
                    self.dash.supported.contains(&e.code)
                } else {
                    true // mode 22 etc.: no bitmap — shown as a candidate, proven by the first reply
                };
                let value = match self.dash.latest.get(&e.id).map(|s| &s.value) {
                    Some(crate::catalog::Value::Num(v)) => {
                        let (dv, du) = self.units.display(*v, &e.unit);
                        format!("{} {du}", crate::ui::dashboard::fmt_value(dv))
                    }
                    Some(crate::catalog::Value::Text(t)) => t.clone(),
                    None => "—".into(),
                };
                PickerRow {
                    id: e.id.clone(),
                    code: e.code.clone(),
                    name: e.name.clone(),
                    value,
                    source: match e.source.as_str() {
                        "j1979" => "J1979".into(),
                        "torque" => "Torque".into(),
                        other => other.to_string(),
                    },
                    supported,
                    on: self.dash.has(&e.id),
                }
            })
            .filter(|r| !self.picker.only_supported || r.supported)
            .collect();
        rows.sort_by(|a, b| (!a.supported, &a.code, &a.id).cmp(&(!b.supported, &b.code, &b.id)));
        rows
    }

    fn select_current(&mut self) -> Vec<Action> {
        if !matches!(self.conn, ConnState::Scanning | ConnState::Idle | ConnState::Failed(_)) {
            return Vec::new();
        }
        let Some(c) = self.connectors.get(self.selected) else { return Vec::new() };
        self.conn = ConnState::Connecting;
        self.facts = DiscoveryFacts::default();
        self.status = format!("Connecting to {}…", c.name);
        vec![Action::Select(c.id.clone())]
    }

    pub fn run_command(&mut self, cmd: &str) -> Vec<Action> {
        match cmd {
            "quit" => {
                self.should_quit = true;
                vec![Action::Quit]
            }
            "help" => {
                self.overlay = Overlay::Help;
                Vec::new()
            }
            "connect" => {
                self.screen = Screen::Connect;
                if self.conn == ConnState::Idle || matches!(self.conn, ConnState::Failed(_)) {
                    self.conn = ConnState::Scanning;
                    self.status = "Scanning for adapters…".into();
                    return vec![Action::Scan];
                }
                Vec::new()
            }
            "rescan" => {
                self.screen = Screen::Connect;
                if !matches!(self.conn, ConnState::Connected | ConnState::Connecting | ConnState::Disconnecting) {
                    self.conn = ConnState::Scanning;
                    self.status = "Scanning for adapters…".into();
                    return vec![Action::Scan];
                }
                Vec::new()
            }
            "disconnect" => {
                if self.conn == ConnState::Connected {
                    self.conn = ConnState::Disconnecting;
                    self.status = "Disconnecting…".into();
                    self.screen = Screen::Connect;
                    return vec![Action::Disconnect];
                }
                Vec::new()
            }
            "support" => {
                self.overlay = Overlay::Support;
                Vec::new()
            }
            "pids" => {
                let mut a = self.go(Screen::Dashboard);
                a.extend(self.rescan_pids());
                a
            }
            "info" => self.go(Screen::VehicleInfo),
            "trouble" => {
                let mut a = self.go(Screen::TroubleCodes);
                if self.dtc.lists.iter().all(Option::is_none) && self.dtc.pending.is_empty() {
                    a.extend(self.read_dtcs());
                }
                a
            }
            "dashboard" => self.go(Screen::Dashboard),
            _ => Vec::new(),
        }
    }

    fn go(&mut self, screen: Screen) -> Vec<Action> {
        if self.screen != screen {
            self.scroll = 0;
        }
        self.screen = screen;
        self.sync_subscriptions()
    }

    /// The app is exiting: stop polling before the link is torn down.
    pub fn on_shutdown(&mut self) -> Vec<Action> {
        self.screen = Screen::Connect;
        self.sync_subscriptions()
    }

    // ---- engine events ----------------------------------------------------

    pub fn on_engine(&mut self, ev: EngineEvent) -> Vec<Action> {
        match ev {
            EngineEvent::Progress { message, detail } => {
                if let Some(s) = humanize(&message, detail.as_deref()) {
                    self.status = s;
                }
                Vec::new()
            }
            EngineEvent::Connectors(list) => {
                self.connectors = list;
                if self.selected >= self.connectors.len() {
                    self.selected = self.connectors.len().saturating_sub(1);
                }
                if self.conn == ConnState::Idle {
                    self.conn = ConnState::Scanning;
                }
                if let Some(key) = self.auto_select.clone() {
                    let want = format!("mock:{key}");
                    if self.conn == ConnState::Scanning {
                        if let Some(i) = self.connectors.iter().position(|c| c.id == want) {
                            self.selected = i;
                            self.auto_select = None;
                            return self.select_current();
                        }
                    }
                }
                Vec::new()
            }
            EngineEvent::Discovery(f) => {
                if f.protocol.is_some() {
                    self.facts.protocol = f.protocol;
                }
                if f.vin.is_some() {
                    self.facts.vin = f.vin;
                }
                if f.pid_count.is_some() {
                    self.facts.pid_count = f.pid_count;
                }
                if f.ecu_count.is_some() {
                    self.facts.ecu_count = f.ecu_count;
                }
                Vec::new()
            }
            EngineEvent::VehicleInfo(vi) => {
                let (_targets, probes) = self.dash.on_vehicle(&vi.vin, &vi.supported_pids, &self.catalog);
                self.after_connect.clear();
                if !probes.is_empty() {
                    self.after_connect.push(Action::Probe(probes));
                }
                self.vehicle = Some(vi);
                Vec::new()
            }
            EngineEvent::Connected => {
                self.conn = ConnState::Connected;
                self.status = "Connected".into();
                // Land on the working view (the Omarchy/TUI convention — btop, lazygit,
                // yazi all open into it); Vehicle Info only when the dashboard would be empty.
                self.screen = if self.dash.cells.is_empty() { Screen::VehicleInfo } else { Screen::Dashboard };
                self.scroll = 0;
                self.subscribed.clear();
                let mut a = self.sync_subscriptions();
                a.extend(std::mem::take(&mut self.after_connect));
                a
            }
            EngineEvent::Disconnected(_) => {
                self.conn = ConnState::Idle;
                self.vehicle = None;
                self.stop_recording_quietly();
                self.dash.on_disconnect();
                self.dtc = DtcState::new();
                self.subscribed.clear();
                self.after_connect.clear();
                self.facts = DiscoveryFacts::default();
                self.status = "Disconnected".into();
                self.screen = Screen::Connect;
                // back to the picker
                self.conn = ConnState::Scanning;
                self.status = "Scanning for adapters…".into();
                vec![Action::Scan]
            }
            EngineEvent::ConnectionFailed(reason) => {
                let text = failure_text(reason.as_deref());
                self.conn = ConnState::Failed(text.clone());
                self.status = text;
                self.screen = Screen::Connect;
                Vec::new()
            }
            EngineEvent::ObdData { command, parsed, .. } => {
                let cmd = command.trim().to_uppercase();
                if let Some(kind) = Kind::from_command(&cmd) {
                    let wmi = self.wmi();
                    let idx = Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
                    self.dtc.lists[idx] = Some(dtc::parse_reply(parsed.as_ref(), &self.dtc_lib, &wmi));
                    self.dtc.pending.retain(|c| c != &cmd);
                    if self.dtc.cursor >= self.dtc.current().len() {
                        self.dtc.cursor = self.dtc.current().len().saturating_sub(1);
                    }
                    return Vec::new();
                }
                if cmd == "04" {
                    let r: ClearResult = dtc::clear_result(parsed.as_ref());
                    self.dtc.pending.retain(|c| c != "04");
                    self.dtc.banner = Some((r.text(), r.ok()));
                    // Re-read so the view reflects vehicle truth — some ECUs re-post hard faults.
                    return self.read_dtcs();
                }
                if self.dash.on_reply(&command, parsed.as_ref(), &mut self.catalog) {
                    self.record_snapshot();
                }
                Vec::new()
            }
        }
    }
}

/// Palette: commands whose name contains `input` as a subsequence, exact prefix first.
pub fn palette_matches(input: &str) -> Vec<&'static str> {
    let needle = input.trim().to_lowercase();
    let mut v: Vec<&'static str> = COMMANDS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| needle.is_empty() || is_subsequence(&needle, n))
        .collect();
    v.sort_by_key(|n| if n.starts_with(&needle) { 0 } else { 1 });
    v
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

fn humanize(message: &str, detail: Option<&str>) -> Option<String> {
    Some(match message {
        "discovering_connectors" => "Scanning for adapters…".into(),
        "scan_round" => return None,
        "connecting" => match detail {
            Some("mock") => "Connecting to mock…".into(),
            Some("demo") => "Starting the demo adapter…".into(),
            Some("ble") => "Opening BLE connection…".into(),
            Some("classic") => "Opening Bluetooth connection…".into(),
            Some("usb") => "Opening USB serial…".into(),
            Some("wifi") => "Opening WiFi link…".into(),
            _ => "Connecting…".into(),
        },
        "initializing_protocol" => "Initializing adapter…".into(),
        "identifying_vehicle" => "Identifying vehicle…".into(),
        "querying_vin" => "Reading VIN…".into(),
        "checking_cache" => "Checking vehicle cache…".into(),
        "cache_hit" => "Vehicle known — using cache".into(),
        "querying_engine_type" => "Reading engine type…".into(),
        "querying_support_pids" => "Reading supported PIDs…".into(),
        "discovering_ecus" => "Discovering ECUs…".into(),
        "saving_cache" => "Saving vehicle cache…".into(),
        "disconnecting" => "Disconnecting…".into(),
        other => other.replace('_', " "),
    })
}

fn failure_text(reason: Option<&str>) -> String {
    match reason {
        Some("vehicle_not_responding") => "Vehicle not responding — is the ignition on?".into(),
        Some("unsupported_protocol") => "Unsupported protocol (not a CAN vehicle)".into(),
        Some("adapter_init_timeout") => "Adapter did not answer during init".into(),
        Some("bt_connect_failed") => "Bluetooth connect failed".into(),
        Some("connect_timeout") => "Adapter did not connect in time".into(),
        Some("ble_connect_timeout") => "Bluetooth adapter did not answer — is it in range and powered?".into(),
        Some("ble_no_serial_service") => "Not an OBD adapter (no serial service)".into(),
        // The notify write is where an adapter that wants a bonded link (OBDLink CX)
        // stalls; after a few failed pairings it goes quiet until it is power-cycled.
        Some(r) if r.starts_with("ble_subscribe") => {
            "Adapter refused the data channel (pairing?) — unplug it for 10 s and retry".into()
        }
        Some("no_connectors") => "No adapters found".into(),
        Some(r) => format!("Connection failed: {}", r.replace('_', " ")),
        None => "Connection failed".into(),
    }
}
