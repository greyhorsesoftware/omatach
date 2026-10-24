//! Trouble Codes screen: three tabs (stored / pending / permanent),
//! a CODE · ECU · DESCRIPTION table, the clear-result banner, and the KOEO
//! confirm modal (the shipped copy, verbatim).
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::dtc::{Kind, KOEO_MESSAGE, KOEO_TITLE};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();

    // tabs
    let mut tabs: Vec<Span> = Vec::new();
    for (i, k) in Kind::ALL.iter().enumerate() {
        let n = app.dtc.lists[i].as_ref().map(|l| l.len().to_string()).unwrap_or_else(|| "·".into());
        let label = if i == app.dtc.tab { format!("‹ {} {} ›", k.title(), n) } else { format!("  {} {}  ", k.title(), n) };
        tabs.push(Span::styled(label, if i == app.dtc.tab { bold } else { dim }));
        tabs.push(Span::raw("  "));
    }
    lines.push(Line::from(tabs));
    lines.push(Line::raw(""));

    let reading = !app.dtc.pending.is_empty();
    match app.dtc.lists[app.dtc.tab].as_deref() {
        None if reading => lines.push(Line::from(Span::styled("reading…", Style::default().fg(Color::Yellow)))),
        None => lines.push(Line::from(Span::styled("press r to read", dim))),
        Some(list) if list.is_empty() => lines.push(Line::from(Span::styled(
            format!("No {} trouble codes", Kind::ALL[app.dtc.tab].title().to_lowercase()),
            Style::default().fg(Color::Green),
        ))),
        Some(list) => {
            lines.push(Line::from(Span::styled(format!("  {:<9}{:<8}{}", "CODE", "ECU", "DESCRIPTION"), dim)));
            for (i, d) in list.iter().enumerate() {
                let sel = i == app.dtc.cursor;
                let marker = if sel { "▸ " } else { "  " };
                let desc = d.description.clone().unwrap_or_else(|| "—".into());
                let text = format!("{marker}{:<7}{:<8}{}", d.code, d.ecu, trunc(&desc, area.width.saturating_sub(20) as usize));
                lines.push(Line::from(Span::styled(text, if sel { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() })));
            }
        }
    }
    if reading && app.dtc.lists[app.dtc.tab].is_some() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled("reading…", Style::default().fg(Color::Yellow))));
    }
    if let Some((text, ok)) = &app.dtc.banner {
        lines.push(Line::raw(""));
        let (mark, color) = if *ok { ("✔", Color::Green) } else { ("⚠", Color::Red) };
        lines.push(Line::from(Span::styled(format!("{mark} {text}"), Style::default().fg(color))));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

pub fn draw_koeo(f: &mut Frame, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let w = area.width.min(62).max(24);
    // height from the wrapped message: text width = w - 2 (border) - 4 (padding)
    let text_w = (w as usize).saturating_sub(6).max(10);
    let msg_lines = wrap_count(KOEO_MESSAGE, text_w) as u16;
    let h = area.height.min(msg_lines + 5);
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, rect);
    let block = Block::bordered().border_style(dim).title(Span::styled(format!(" {KOEO_TITLE} "), Style::default().add_modifier(Modifier::BOLD)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [_, msg_a, btn_a] = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(2)]).areas(inner);
    let msg_a = Rect { x: msg_a.x + 2, width: msg_a.width.saturating_sub(4), ..msg_a };
    f.render_widget(Paragraph::new(KOEO_MESSAGE).wrap(Wrap { trim: true }), msg_a);
    let buttons = Line::from(vec![
        Span::raw("        "),
        Span::styled("Cancel", dim),
        Span::raw("  (esc)          "),
        Span::styled("‹ Clear Codes ›", Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)),
        Span::styled("  (↵)", dim),
    ]);
    let btn_rect = Rect { y: btn_a.y + btn_a.height.saturating_sub(1), height: 1, ..btn_a };
    f.render_widget(Paragraph::new(buttons), btn_rect);
}

/// Word-wrap line count for `text` at `width` (mirrors ratatui's word wrap closely enough for sizing).
fn wrap_count(text: &str, width: usize) -> usize {
    let mut lines = 1;
    let mut col = 0;
    for word in text.split(' ') {
        let wl = word.chars().count();
        if col == 0 {
            col = wl;
        } else if col + 1 + wl > width {
            lines += 1;
            col = wl;
        } else {
            col += 1 + wl;
        }
    }
    lines
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
    use crate::app::{Action, App, ConnState, Overlay, Screen};
    use crate::engine::EngineEvent;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn parsed(ecu: &str, raw: &str, bytes: &[u8]) -> serde_json::Value {
        serde_json::json!({ "type": "trouble_codes", "all_controllers": { ecu: { "raw_hex": raw, "data_bytes": bytes, "controller_id": ecu, "is_valid": !bytes.is_empty() } } })
    }

    fn obd(cmd: &str, p: serde_json::Value) -> EngineEvent {
        EngineEvent::ObdData { command: cmd.into(), data: String::new(), error: None, parsed: Some(p) }
    }

    fn render(app: &App) -> String {
        let mut t = Terminal::new(TestBackend::new(90, 22)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { out.push_str(buf[(x, y)].symbol()); }
            out.push('\n');
        }
        out
    }

    /// `cargo test print_dtc_demo -- --nocapture --ignored` → the mockups.
    #[test]
    #[ignore]
    fn print_dtc_demo() {
        let mut app = App::new(None);
        app.conn = ConnState::Connected;
        app.vehicle = Some(Box::new(obd_session_rs::vehicle_info::VehicleInfo {
            vin: "OMA11BIT000000001".into(), wmi: "OMA".into(), manufacturer: None,
            engine_type: obd_session_rs::vehicle_info::EngineType::Gasoline, ecus: vec![],
            supported_pids: vec![], protocol: "ISO 15765-4 CAN 11-bit (500k)".into(),
            year_char: None, pcm_strategy: None, module_census: vec![], strategy_probed: None,
        }));
        app.on_key(key(KeyCode::Char('t')));
        app.on_engine(obd("03", parsed("7E8", "43 02 03 00 01 71", &[2, 0x03, 0x00, 0x01, 0x71])));
        app.on_engine(obd("07", parsed("7E8", "47 01 04 20", &[1, 0x04, 0x20])));
        app.on_engine(obd("0A", parsed("7E8", "4A 01 03 00", &[1, 0x03, 0x00])));
        println!("\n== trouble codes 90x22 ==\n{}", render(&app));
        app.on_key(key(KeyCode::Char('x')));
        println!("\n== KOEO modal 90x22 ==\n{}", render(&app));
        app.on_key(key(KeyCode::Enter));
        app.on_engine(obd("04", parsed("7E8", "7F 04 22", &[0x04, 0x22])));
        app.on_engine(obd("03", parsed("7E8", "43 02 03 00 01 71", &[2, 0x03, 0x00, 0x01, 0x71])));
        app.on_engine(obd("07", parsed("7E8", "47 00", &[0])));
        app.on_engine(obd("0A", parsed("7E8", "4A 00", &[0])));
        println!("\n== refused 90x22 ==\n{}", render(&app));
    }

    #[test]
    fn read_show_clear_and_reread_flow() {
        let mut app = App::new(None);
        app.conn = ConnState::Connected;
        // entering the screen issues the three reads
        assert_eq!(app.on_key(key(KeyCode::Char('t'))), vec![Action::Probe(vec!["03".into(), "07".into(), "0A".into()])]);
        assert_eq!(app.screen, Screen::TroubleCodes);
        assert!(render(&app).contains("reading…"));
        app.on_engine(obd("03", parsed("7E8", "43 02 03 00 01 71", &[2, 0x03, 0x00, 0x01, 0x71])));
        app.on_engine(obd("07", parsed("7E8", "47 00", &[0])));
        app.on_engine(EngineEvent::ObdData { command: "0A".into(), data: "NO DATA".into(), error: None, parsed: None });
        assert!(app.dtc.pending.is_empty());
        let s = render(&app);
        assert!(s.contains("‹ Stored 2 ›") && s.contains("P0300") && s.contains("P0171") && s.to_lowercase().contains("misfire"), "{s}");
        // tabs
        app.on_key(key(KeyCode::Tab));
        assert!(render(&app).contains("No pending trouble codes"));
        app.on_key(key(KeyCode::Tab));
        assert!(render(&app).contains("No permanent trouble codes"));
        app.on_key(key(KeyCode::Tab));
        // clear: modal, confirm → 04, refusal banner
        app.on_key(key(KeyCode::Char('x')));
        assert_eq!(app.overlay, Overlay::Koeo);
        assert!(render(&app).contains("Key On, Engine Off"));
        assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Action::Probe(vec!["04".into()])]);
        assert_eq!(app.on_engine(obd("04", parsed("7E8", "7F 04 22", &[0x04, 0x22]))), vec![Action::Probe(vec!["03".into(), "07".into(), "0A".into()])]);
        assert!(render(&app).contains("turn the engine off"), "{}", render(&app));
        // cleared for real → banner + re-read shows none
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Char('y')));
        app.on_engine(obd("04", parsed("7E8", "44", &[])));
        app.on_engine(obd("03", parsed("7E8", "43 00", &[0])));
        let s = render(&app);
        assert!(s.contains("✔ Trouble codes cleared") && s.contains("No stored trouble codes"), "{s}");
        // esc cancels the modal
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
    }
}
