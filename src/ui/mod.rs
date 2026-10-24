//! ratatui rendering: side-borderless, header/footer rules stand in
//! for the tile edges, ANSI-16 colors only (the terminal palette recolors
//! on Omarchy theme switch for free).
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

pub mod dashboard;
pub mod dtc;
pub mod support;
pub mod vehicle;

use crate::app::{palette_matches, App, ConnState, Overlay, Screen, COMMANDS};

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    f.render_widget(header_line(app, header.width), header);
    match app.screen {
        Screen::Connect => draw_connect(f, app, body),
        Screen::VehicleInfo if app.vehicle.is_some() => vehicle::draw(f, app, pad(body, 2, 1)),
        Screen::VehicleInfo => draw_placeholder(f, app, body, "Vehicle Info", "is waiting for identify"),
        Screen::TroubleCodes if app.is_connected() => dtc::draw(f, app, pad(body, 2, 1)),
        Screen::TroubleCodes => draw_placeholder(f, app, body, "Trouble Codes", ""),
        Screen::Dashboard => dashboard::draw(f, app, pad(body, 1, 1)),
    }
    f.render_widget(footer_line(app, footer.width), footer);
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(f, app, body),
        Overlay::Palette { input } => draw_palette(f, input, body),
        Overlay::Picker => dashboard::draw_picker(f, app, body),
        Overlay::Koeo => dtc::draw_koeo(f, body),
        Overlay::Support => support::draw(f, body),
        Overlay::RecordingSaved(s) => draw_recording_saved(f, s, body),
    }
}

// ---- chrome ---------------------------------------------------------------

fn header_line(app: &App, width: u16) -> Paragraph<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let title = match &app.vehicle {
        Some(v) if !v.vin.is_empty() => v.vin.clone(),
        _ => app.screen.title().to_string(),
    };
    let proto = app
        .vehicle
        .as_ref()
        .map(|v| v.protocol.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| app.facts.protocol.clone())
        .map(|p| short_protocol(&p));
    let (dot, state, color) = match &app.conn {
        ConnState::Connected => ("●", "connected", Color::Green),
        ConnState::Connecting => ("◐", "connecting…", Color::Yellow),
        ConnState::Scanning => ("○", "scanning…", Color::Yellow),
        ConnState::Disconnecting => ("◌", "disconnecting…", Color::Yellow),
        ConnState::Failed(_) => ("✕", "failed", Color::Red),
        ConnState::Idle => ("○", "idle", Color::DarkGray),
    };
    let mut left = format!("─── omatach · {title}");
    if let Some(p) = proto {
        left.push_str(&format!(" · {p}"));
    }
    left.push(' ');
    let right = format!(" {dot} {state} ───");
    let fill = (width as usize).saturating_sub(left.chars().count() + right.chars().count());
    Paragraph::new(Line::from(vec![
        Span::styled(left, dim),
        Span::styled("·".repeat(fill), dim),
        Span::styled(format!(" {dot} "), Style::default().fg(color)),
        Span::styled(format!("{state} ───"), dim),
    ]))
}

fn short_protocol(p: &str) -> String {
    // "ISO 15765-4 CAN 11-bit (500k)" → "CAN 11/500"
    let bits = if p.contains("29-bit") { "29" } else { "11" };
    let kb = if p.contains("250k") { "250" } else { "500" };
    format!("CAN {bits}/{kb}")
}

fn footer_line(app: &App, width: u16) -> Paragraph<'static> {
    let hints: &[(&str, &str)] = match (&app.overlay, app.screen) {
        (Overlay::Help, _) => &[("any key", "close")],
        (Overlay::Palette { .. }, _) => &[("type", "filter"), ("↵", "run"), ("esc", "close")],
        (Overlay::Koeo, _) => &[("↵", "clear codes"), ("esc", "cancel")],
        (Overlay::Support, _) => &[("o", "open in browser"), ("esc", "close")],
        (Overlay::RecordingSaved(_), _) => &[("any key", "close")],
        (_, Screen::Dashboard) if app.is_recording() => &[("R", "stop recording"), ("+", "add"), ("-", "remove"), ("s", "style"), ("i", "info"), ("t", "trouble"), ("?", "help"), ("q", "quit")],
        (_, Screen::TroubleCodes) if app.is_connected() => &[("⇥", "switch list"), ("x", "clear codes"), ("r", "refresh"), ("d", "dash"), ("i", "info"), ("$", "support"), ("?", "help"), ("q", "quit")],
        (Overlay::Picker, _) => &[("type", "filter"), ("↑↓", "move"), ("space", "add/remove"), ("⇥", "supported·all"), ("↵/esc", "done")],
        (_, Screen::Dashboard) if app.is_connected() => &[("+", "add"), ("-", "remove"), ("←→", "select"), ("⇧←→", "reorder"), ("s", "style"), ("u", "units"), ("R", "record"), ("i", "info"), ("t", "trouble"), ("$", "support"), ("?", "help"), ("q", "quit")],
        (_, Screen::Connect) => match app.conn {
            ConnState::Connecting => &[("esc", "cancel"), ("$", "support"), ("?", "help"), ("q", "quit")],
            ConnState::Connected => &[("x", "disconnect"), ("i", "info"), ("t", "trouble"), ("d", "dash"), ("$", "support"), ("?", "help"), ("q", "quit")],
            _ => &[("↑↓", "move"), ("↵", "connect"), ("r", "rescan"), ("/", "find"), ("$", "support"), ("?", "help"), ("q", "quit")],
        },
        (_, Screen::VehicleInfo) if app.vehicle.is_some() => &[("i", "info"), ("t", "trouble"), ("d", "dash"), ("j/k", "scroll"), ("/", "find"), ("$", "support"), ("?", "help"), ("q", "quit")],
        _ => &[("i", "info"), ("t", "trouble"), ("d", "dash"), ("c", "connect"), ("/", "find"), ("$", "support"), ("?", "help"), ("q", "quit")],
    };
    // Bottom-right: the recording orb (reserved first — it is never dropped),
    // then the dim status, then the key hints from the left.
    let rec = app.recording_status();
    let rec_w = rec.as_ref().map(|(_, t)| t.chars().count() + 3).unwrap_or(0); // "● " + text + " "
    let avail = (width as usize).saturating_sub(rec_w);
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let mut used: usize = 1;
    for (k, what) in hints {
        let w = k.chars().count() + what.chars().count() + 5;
        if used + w > avail {
            break; // hints yield to the orb on a narrow tile
        }
        spans.push(Span::styled(k.to_string(), Style::default().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {what}    "), Style::default().fg(Color::DarkGray)));
        used += w;
    }
    if let Some(status) = app.footer_status() {
        let need = status.chars().count() + 2;
        if used + need <= avail {
            spans.push(Span::raw(" ".repeat(avail - used - need + 1)));
            spans.push(Span::styled(status.clone(), Style::default().fg(Color::DarkGray)));
            used = avail - 1;
        }
    }
    if let Some((on, text)) = rec {
        if used < avail {
            spans.push(Span::raw(" ".repeat(avail - used)));
        }
        let (orb, style) = if on { ("●", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)) } else { ("○", Style::default().fg(Color::Red)) };
        spans.push(Span::styled(orb.to_string(), style));
        spans.push(Span::styled(format!(" {text} "), Style::default().fg(Color::Red)));
    }
    Paragraph::new(Line::from(spans))
}

// ---- connect screen ---------------------------------------------------------

fn draw_connect(f: &mut Frame, app: &App, area: Rect) {
    let inner = pad(area, 2, 1);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled("Select an adapter", Style::default().add_modifier(Modifier::BOLD))));
    lines.push(Line::raw(""));
    if app.connectors.is_empty() {
        lines.push(Line::from(Span::styled("  (no adapters yet)", Style::default().fg(Color::DarkGray))));
    }
    for (i, c) in app.connectors.iter().enumerate() {
        let sel = i == app.selected;
        let marker = if sel { "▸ " } else { "  " };
        let kind = match c.connector_type.as_str() {
            "ble" => "BLE",
            "classic" => "BT classic",
            "usb" => "USB",
            "wifi" => "WiFi",
            "mock" => "mock",
            "demo" => "demo",
            other => other,
        };
        let text = format!("{marker}{:<24}{:<12}{}", trunc(&c.name, 23), kind, trunc(&c.id, 34));
        let style = if sel { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(Span::styled(text, style)));
    }
    lines.push(Line::raw(""));
    match &app.conn {
        ConnState::Scanning => lines.push(Line::from(Span::styled(
            "scanning  bluetooth · usb · wifi …",
            Style::default().fg(Color::DarkGray),
        ))),
        ConnState::Connecting | ConnState::Disconnecting => {
            lines.push(Line::from(Span::styled(app.status.clone(), Style::default().fg(Color::Yellow))));
            let f = &app.facts;
            let mut facts: Vec<String> = Vec::new();
            if let Some(p) = &f.protocol {
                facts.push(p.clone());
            }
            if let Some(v) = &f.vin {
                facts.push(format!("VIN {v}"));
            }
            if let Some(n) = f.pid_count {
                facts.push(format!("{n} PIDs"));
            }
            if let Some(n) = f.ecu_count {
                facts.push(format!("{n} ECUs"));
            }
            if !facts.is_empty() {
                lines.push(Line::from(Span::styled(format!("  {}", facts.join(" · ")), Style::default().fg(Color::DarkGray))));
            }
        }
        ConnState::Connected => lines.push(Line::from(Span::styled(
            format!("{}  —  x to disconnect", app.status),
            Style::default().fg(Color::Green),
        ))),
        ConnState::Failed(text) => lines.push(Line::from(Span::styled(text.clone(), Style::default().fg(Color::Red)))),
        ConnState::Idle => {}
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("omatach is free & donationware — ", Style::default().fg(Color::DarkGray)),
        Span::styled("$", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" to support ♥", Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ---- placeholders -----------------------------------------------------------

fn draw_placeholder(f: &mut Frame, app: &App, area: Rect, name: &str, when: &str) {
    let inner = pad(area, 2, 1);
    let text = if app.is_connected() {
        vec![
            Line::from(Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD))),
            Line::raw(""),
            Line::from(Span::styled(format!("{name} {when}."), Style::default().fg(Color::DarkGray))),
        ]
    } else {
        vec![
            Line::from(Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD))),
            Line::raw(""),
            Line::from(Span::styled("Not connected — press c to pick an adapter.", Style::default().fg(Color::DarkGray))),
        ]
    };
    f.render_widget(Paragraph::new(text), inner);
}

// ---- overlays -------------------------------------------------------------

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let rows: &[(&str, &str)] = &[
        ("c", "connect / adapter picker"),
        ("i  t  d", "vehicle info · trouble codes · dashboard"),
        ("/", "command palette"),
        ("↑↓  j k", "move"),
        ("↵", "select / connect"),
        ("r", "rescan adapters"),
        ("x", "disconnect"),
        ("+  -", "dashboard: add / remove a gauge"),
        ("←→ ⇧←→", "dashboard: select / reorder"),
        ("s  u  r", "dashboard: gauge style / units / rescan pids/"),
        ("R", "dashboard: start / stop recording (CSV, RevOBD format)"),
        ("⇥  x  r", "trouble codes: switch list / clear / refresh"),
        ("esc", "cancel a connect · close"),
        ("$", "support / donate"),
        ("?", "this help"),
        ("q", "quit"),
    ];
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(" omatach keys ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(Color::DarkGray)),
        ]),
        Line::raw(""),
    ];
    for (k, what) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("   {k:<9}"), Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(what.to_string()),
        ]));
    }
    if let Some(p) = &app.log_path {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(format!("   log  {p}"), Style::default().fg(Color::DarkGray))));
    }
    modal(f, area, 52, (lines.len() + 2) as u16, lines);
}

fn draw_palette(f: &mut Frame, input: &str, area: Rect) {
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(" / ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(input.to_string()),
        Span::styled("▏", Style::default().fg(Color::DarkGray)),
    ])];
    for (i, name) in palette_matches(input).iter().enumerate() {
        let desc = COMMANDS.iter().find(|(n, _)| n == name).map(|(_, d)| *d).unwrap_or("");
        let style = if i == 0 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(Span::styled(format!("   {name:<12}{desc}"), style)));
    }
    let h = (lines.len() + 2) as u16;
    let w = 48u16.min(area.width);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let rect = Rect { x, y: area.y + 1, width: w, height: h.min(area.height) };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(ratatui::widgets::Block::bordered().border_style(Style::default().fg(Color::DarkGray))),
        rect,
    );
}

fn draw_recording_saved(f: &mut Frame, s: &crate::recorder::Summary, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = s.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let lines: Vec<Line> = vec![
        Line::from(Span::styled(" Recording saved ", Style::default().add_modifier(Modifier::BOLD))),
        Line::raw(""),
        Line::from(vec![Span::styled("   file      ", dim), Span::styled(name, Style::default().fg(Color::Cyan))]),
        Line::from(vec![Span::styled("   folder    ", dim), Span::raw(folder)]),
        Line::from(vec![Span::styled("   rows      ", dim), Span::raw(format!("{}", s.rows))]),
        Line::from(vec![Span::styled("   duration  ", dim), Span::raw(format!("{:.0} s", s.seconds))]),
        Line::from(vec![Span::styled("   columns   ", dim), Span::raw(format!("{}  (Time (s) + {} signals)", s.columns + 1, s.columns))]),
        Line::raw(""),
        Line::from(Span::styled("   wide CSV in RevOBD's native (LiveLink) shape — opens in RevOBD's chart view", dim)),
    ];
    let w = (lines.iter().map(|l| l.width()).max().unwrap_or(40) as u16 + 4).min(area.width);
    modal(f, area, w, (lines.len() + 2) as u16, lines);
}

fn modal(f: &mut Frame, area: Rect, w: u16, h: u16, lines: Vec<Line>) {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .block(ratatui::widgets::Block::bordered().border_style(Style::default().fg(Color::DarkGray))),
        rect,
    );
}

// ---- helpers --------------------------------------------------------------

fn pad(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x + x.min(area.width / 2),
        y: area.y + y.min(area.height / 2),
        width: area.width.saturating_sub(2 * x),
        height: area.height.saturating_sub(2 * y),
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App, ConnState, Screen};
    use crate::engine::EngineEvent;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use obd_session_rs::platform::ConnectorInfo;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn mocks() -> Vec<ConnectorInfo> {
        vec![
            ConnectorInfo { id: "mock:11bit".into(), name: "Mock 11-bit (J1979)".into(), connector_type: "mock".into() },
            ConnectorInfo { id: "mock:29bit".into(), name: "Mock 29-bit (J1979)".into(), connector_type: "mock".into() },
        ]
    }

    #[test]
    fn launch_scans_and_connect_flow_reaches_vehicle_info() {
        let mut app = App::new(None);
        assert_eq!(app.run_command("connect"), vec![Action::Scan]);
        assert_eq!(app.conn, ConnState::Scanning);
        assert!(app.on_engine(EngineEvent::Connectors(mocks())).is_empty());
        let s = render(&app, 80, 24);
        assert!(s.contains("Select an adapter"), "{s}");
        assert!(s.contains("▸ Mock 11-bit"), "{s}");
        assert!(s.contains("scanning"), "{s}");
        // move down, connect the 29-bit mock
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::Select("mock:29bit".into())]);
        assert_eq!(app.conn, ConnState::Connecting);
        app.on_engine(EngineEvent::Progress { message: "querying_vin".into(), detail: None });
        assert_eq!(app.status, "Reading VIN…");
        app.on_engine(EngineEvent::Discovery(crate::engine::DiscoveryFacts { vin: Some("OMA29BIT000000002".into()), ..Default::default() }));
        let s = render(&app, 80, 24);
        assert!(s.contains("Reading VIN…") && s.contains("VIN OMA29BIT000000002"), "{s}");
        assert!(app.on_engine(EngineEvent::Connected).is_empty());
        assert_eq!(app.conn, ConnState::Connected);
        assert_eq!(app.screen, Screen::VehicleInfo, "no dashboard cells (no vehicle_info arrived) → info is the landing");
        let s = render(&app, 80, 24);
        assert!(s.contains("● connected"), "{s}");
        // navigate the three sections
        app.on_key(key(KeyCode::Char('t')));
        assert_eq!(app.screen, Screen::TroubleCodes);
        app.on_key(key(KeyCode::Char('d')));
        assert_eq!(app.screen, Screen::Dashboard);
        app.on_key(key(KeyCode::Char('i')));
        assert_eq!(app.screen, Screen::VehicleInfo);
        // disconnect from the picker
        app.on_key(key(KeyCode::Char('c')));
        assert_eq!(app.on_key(key(KeyCode::Char('x'))), vec![Action::Disconnect]);
        assert_eq!(app.on_engine(EngineEvent::Disconnected(None)), vec![Action::Scan]);
        assert_eq!(app.conn, ConnState::Scanning);
    }

    #[test]
    fn auto_select_picks_the_requested_mock() {
        let mut app = App::new(None);
        app.auto_select = Some("29bit".into());
        app.run_command("connect");
        assert_eq!(app.on_engine(EngineEvent::Connectors(mocks())), vec![Action::Select("mock:29bit".into())]);
        assert_eq!(app.auto_select, None);
    }

    #[test]
    fn failure_is_humanized_and_retry_rescans() {
        let mut app = App::new(None);
        app.run_command("connect");
        app.on_engine(EngineEvent::Connectors(mocks()));
        app.on_key(key(KeyCode::Enter));
        app.on_engine(EngineEvent::ConnectionFailed(Some("vehicle_not_responding".into())));
        assert!(matches!(app.conn, ConnState::Failed(_)));
        let s = render(&app, 80, 24);
        assert!(s.contains("ignition on"), "{s}");
        assert_eq!(app.on_key(key(KeyCode::Char('r'))), vec![Action::Scan]);
    }

    #[test]
    fn reflows_to_a_small_tile_without_panicking() {
        let mut app = App::new(None);
        app.run_command("connect");
        app.on_engine(EngineEvent::Connectors(mocks()));
        for (w, h) in [(80, 24), (40, 12), (20, 6), (120, 40), (3, 3)] {
            let s = render(&app, w, h);
            assert_eq!(s.lines().count(), h as usize);
            if w >= 20 {
                assert!(s.contains("omatach"), "{w}x{h}: {s}");
            }
        }
        app.overlay = crate::app::Overlay::Help;
        let _ = render(&app, 30, 8);
        app.overlay = crate::app::Overlay::Palette { input: "tr".into() };
        let s = render(&app, 60, 20);
        assert!(s.contains("trouble"), "{s}");
    }

    #[test]
    fn torque_csv_import_shadows_computes_and_syncs_subscriptions() {
        let tmp = std::env::temp_dir().join(format!("omatach-app-{}", std::process::id()));
        let pids = tmp.join("pids");
        std::fs::create_dir_all(&pids).unwrap();
        std::fs::write(pids.join("t.csv"), "Name,ShortName,ModeAndPID,Equation,Min Value,Max Value,Units,Header\n\
Coolant Fahrenheit,ECTF,0x0105,A*1.8-40,-40,420,°F,\n\
Double RPM,DRPM,,VAL{RPM}*2,0,20000,rpm,\n\
Trans Temp,TFT,0x221E1C,A-40,-40,215,°C,7E1\n\
Bad,B,0x220100,A +* B,0,1,,7E0\n").unwrap();
        let mut app = App::new(Some(&tmp));
        assert!(app.rescan_pids().is_empty(), "not connected → no subscription actions");
        assert!(app.pids_banner.as_deref().unwrap_or("").contains("3 PIDs (1 computed), 1 row skipped"), "{:?}", app.pids_banner);
        // shadowing: the CSV 0105 replaces the built-in datapoint
        let ect = app.catalog.get("0105.1").unwrap();
        assert_eq!((ect.unit.as_str(), ect.source.as_str()), ("°F", "torque"));
        assert!(app.catalog.get("COMP:Double RPM").map_or(false, |e| e.computed));
        assert_eq!(app.catalog.get("221E1C@7E1.1").unwrap().header.as_deref(), Some("7E1"));
        let c = app.catalog.closure("COMP:Double RPM");
        assert_eq!(c.inputs, vec![("010C".to_string(), None)]);
        assert!(c.missing.is_empty() && !c.cycle);
        // connected: adding the computed entry subscribes its real input; the headered one carries its header
        app.conn = ConnState::Connected;
        app.screen = crate::app::Screen::Dashboard;
        app.dash.supported = ["010C", "0105"].iter().map(|s| s.to_string()).collect();
        app.overlay = crate::app::Overlay::Picker;
        app.picker.input = "double".into();
        let rows = app.picker_rows();
        assert_eq!((rows[0].name.as_str(), rows[0].source.as_str(), rows[0].supported), ("Double RPM", "Torque", true), "{rows:?}");
        assert_eq!(app.on_key(key(KeyCode::Char(' '))), vec![Action::Subscribe(vec![("010C".into(), None)])]);
        app.picker.input = "trans".into();
        app.picker.cursor = 0;
        assert_eq!(app.on_key(key(KeyCode::Char(' '))), vec![Action::Subscribe(vec![("221E1C".into(), Some("7E1".into()))])]);
        // a reply for RPM drives the computed cell
        let p = serde_json::json!({ "all_controllers": { "7E8": { "raw_hex": "41 0C 0B B8", "data_bytes": [0x0B, 0xB8], "controller_id": "7E8", "is_valid": true } } });
        app.on_engine(EngineEvent::ObdData { command: "010C".into(), data: String::new(), error: None, parsed: Some(p) });
        assert_eq!(app.dash.latest["COMP:Double RPM"].value, crate::catalog::Value::Num(1500.0));
        // removing the CSV un-shadows the built-in and drops the orphaned subscription
        std::fs::remove_file(pids.join("t.csv")).unwrap();
        let acts = app.rescan_pids();
        assert_eq!(app.catalog.get("0105.1").unwrap().unit, "°C");
        assert!(acts.iter().any(|a| matches!(a, Action::Unsubscribe(v) if v.iter().any(|(c, _)| c == "221E1C"))), "{acts:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn support_modal_renders_qr_and_opens_the_link() {
        let mut app = App::new(None);
        assert!(app.on_key(key(KeyCode::Char('$'))).is_empty());
        assert_eq!(app.overlay, crate::app::Overlay::Support);
        let s = render(&app, 100, 30);
        assert!(s.contains("Support omatach") && s.contains("donationware") && s.contains("paypal.com/donate"), "{s}");
        assert!(crate::support::SUPPORT_URL.contains("hosted_button_id=VRTDT7PVAFARL"));
        assert!(s.contains('█') || s.contains('▀') || s.contains('▄'), "QR blocks\n{s}");
        assert_eq!(app.on_key(key(KeyCode::Char('o'))), vec![Action::OpenUrl(crate::support::SUPPORT_URL.into())]);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, crate::app::Overlay::None);
        // narrow tile: text only, no panic
        let mut app = App::new(None);
        app.overlay = crate::app::Overlay::Support;
        let s = render(&app, 40, 14);
        assert!(s.contains("Support"), "{s}");
    }

    #[test]
    fn record_toggle_writes_rows_and_shows_the_popup() {
        let tmp = std::env::temp_dir().join(format!("omatach-recapp-{}", std::process::id()));
        let mut app = App::new(None);
        app.recordings_dir = Some(tmp.join("recordings"));
        app.conn = ConnState::Connected;
        app.dash.supported = ["010C", "0105"].iter().map(|s| s.to_string()).collect();
        app.dash.cells = vec![crate::dash::Cell { id: "0105.1".into(), style: crate::dash::GaugeStyle::Bar }, crate::dash::Cell { id: "010C.1".into(), style: crate::dash::GaugeStyle::Dial }];
        app.screen = Screen::Dashboard;
        app.on_key(key(KeyCode::Char('R')));
        assert!(app.is_recording());
        let s = render(&app, 100, 30);
        assert!(s.contains("● REC 00:00 · 0 rows") && s.trim_end().ends_with("rows"), "orb bottom-right\n{s}");
        let p = |ecu: &str, raw: &str, bytes: &[u8]| serde_json::json!({ "all_controllers": { ecu: { "raw_hex": raw, "data_bytes": bytes, "controller_id": ecu, "is_valid": true } } });
        app.on_engine(EngineEvent::ObdData { command: "010C".into(), data: String::new(), error: None, parsed: Some(p("7E8", "41 0C 0B B8", &[0x0B, 0xB8])) });
        app.on_engine(EngineEvent::ObdData { command: "0105".into(), data: String::new(), error: None, parsed: Some(p("7E8", "41 05 83", &[0x83])) });
        assert_eq!(app.recorder.as_ref().unwrap().rows, 2);
        app.on_key(key(KeyCode::Char('R')));
        assert!(!app.is_recording());
        let crate::app::Overlay::RecordingSaved(s) = app.overlay.clone() else { panic!("popup expected: {:?}", app.overlay) };
        assert_eq!((s.rows, s.columns), (2, 2));
        let out = render(&app, 100, 30);
        assert!(out.contains("Recording saved") && out.contains("recording_20") && out.contains("rows"), "{out}");
        let text = std::fs::read_to_string(&s.path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Time (s),RPM (rpm),ECT (°C)");
        assert!(lines[1].starts_with("0.000,750,"), "{}", lines[1]);
        assert!(lines[2].ends_with(",750,91"), "{}", lines[2]);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, crate::app::Overlay::None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn connect_lands_on_the_dashboard_when_it_has_cells() {
        let mut app = App::new(None);
        app.run_command("connect");
        app.on_engine(EngineEvent::Connectors(mocks()));
        app.on_key(key(KeyCode::Enter));
        let vi = obd_session_rs::vehicle_info::VehicleInfo {
            vin: "OMA11BIT000000001".into(), wmi: "OMA".into(), manufacturer: None,
            engine_type: obd_session_rs::vehicle_info::EngineType::Gasoline, ecus: vec![],
            supported_pids: vec!["010C".into(), "010D".into(), "0105".into()], protocol: "ISO 15765-4 CAN 11-bit (500k)".into(),
            year_char: None, pcm_strategy: None, module_census: vec![], strategy_probed: None,
        };
        app.on_engine(EngineEvent::VehicleInfo(Box::new(vi)));
        let acts = app.on_engine(EngineEvent::Connected);
        assert_eq!(app.screen, Screen::Dashboard);
        assert!(acts.iter().any(|a| matches!(a, Action::Subscribe(_))), "{acts:?}");
        let s = render(&app, 100, 30);
        assert!(s.contains("OMA11BIT000000001") && s.contains("RPM") && s.contains("i info"), "{s}");
    }

    /// Only the Dashboard polls: leaving it unsubscribes everything, coming
    /// back resubscribes, and quitting stops the polling before teardown.
    #[test]
    fn polling_follows_the_dashboard() {
        let mut app = App::new(None);
        app.run_command("connect");
        app.on_engine(EngineEvent::Connectors(mocks()));
        app.on_key(key(KeyCode::Enter));
        let vi = obd_session_rs::vehicle_info::VehicleInfo {
            vin: "OMA11BIT000000001".into(), wmi: "OMA".into(), manufacturer: None,
            engine_type: obd_session_rs::vehicle_info::EngineType::Gasoline, ecus: vec![],
            supported_pids: vec!["010C".into(), "010D".into(), "0105".into()], protocol: "ISO 15765-4 CAN 11-bit (500k)".into(),
            year_char: None, pcm_strategy: None, module_census: vec![], strategy_probed: None,
        };
        app.on_engine(EngineEvent::VehicleInfo(Box::new(vi)));
        let on_connect = app.on_engine(EngineEvent::Connected);
        let polled = on_connect
            .iter()
            .find_map(|a| if let Action::Subscribe(v) = a { Some(v.clone()) } else { None })
            .expect("dashboard subscribes on connect");

        for cmd in ["info", "trouble"] {
            let leaving = app.run_command(cmd);
            assert!(leaving.contains(&Action::Unsubscribe(polled.clone())), "{cmd}: {leaving:?}");
            assert!(!leaving.iter().any(|a| matches!(a, Action::Subscribe(_))), "{cmd}: {leaving:?}");
            let back = app.run_command("dashboard");
            assert_eq!(back, vec![Action::Subscribe(polled.clone())], "back from {cmd}");
        }
        // Moving between two non-dashboard screens changes nothing on the wire.
        app.run_command("info");
        assert!(!app.run_command("trouble").iter().any(|a| matches!(a, Action::Subscribe(_) | Action::Unsubscribe(_))));

        app.run_command("dashboard");
        assert_eq!(app.on_shutdown(), vec![Action::Unsubscribe(polled)]);
    }

    #[test]
    fn palette_runs_the_first_match() {
        let mut app = App::new(None);
        app.on_key(key(KeyCode::Char('/')));
        for c in "tro".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(palette_matches("tro"), vec!["trouble"]);
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::TroubleCodes);
        app.on_key(key(KeyCode::Char('/')));
        app.on_key(key(KeyCode::Char('q')));
        assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::Quit]);
        assert!(app.should_quit);
    }
}
