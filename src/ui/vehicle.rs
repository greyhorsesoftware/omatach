//! Vehicle Info: a read-only render of the engine's `vehicle_info`,
//! all J1979 — VIN (09 02), protocol/addressing, ECU table (09 0A names with
//! conventional fallbacks — many real cars don't answer 09 0A), calibration ids
//! (09 04), and the supported-PID roster grouped by mode. `j/k` scroll.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use obd_session_rs::vehicle_info::{EngineType, VehicleInfo};

use crate::app::App;

/// ISO 15765-4 conventional module names when the car doesn't say (09 0A).
/// 11-bit keys are the request ids the engine uses (`7E0`…); 29-bit keys are the
/// source-address byte (`10`…). Only the two everyone agrees on are named.
pub fn conventional_name(controller_id: &str) -> Option<&'static str> {
    match controller_id.to_ascii_uppercase().as_str() {
        "7E0" | "10" => Some("Engine (ECM/PCM)"),
        "7E1" | "18" => Some("Transmission (TCM)"),
        _ => None,
    }
}

pub fn is_29bit(protocol: &str) -> bool {
    protocol.contains("29-bit")
}

fn engine_type(e: EngineType) -> &'static str {
    match e {
        EngineType::Gasoline => "Gasoline",
        EngineType::Diesel => "Diesel",
        EngineType::Unknown => "unknown",
    }
}

/// Build the screen's lines; `width` drives the PID grid.
pub fn lines(v: &VehicleInfo, width: u16) -> Vec<Line<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let kv = |k: &str, val: String| Line::from(vec![Span::styled(format!("  {k:<14}"), dim), Span::raw(val)]);

    let mut out: Vec<Line<'static>> = Vec::new();
    out.push(Line::from(Span::styled("VEHICLE", bold)));
    out.push(kv("VIN", if v.vin.is_empty() { "—".into() } else { v.vin.clone() }));
    let maker = match (&v.manufacturer, v.wmi.is_empty()) {
        (Some(m), _) => format!("{m}  ({})", v.wmi),
        (None, false) => format!("WMI {}", v.wmi),
        _ => "—".into(),
    };
    out.push(kv("Manufacturer", maker));
    if let Some(y) = &v.year_char {
        out.push(kv("Model year", format!("VIN position 10 = {y}")));
    }
    out.push(kv("Engine", engine_type(v.engine_type).to_string()));
    let protocol = if v.protocol.is_empty() { "—".to_string() } else { v.protocol.clone() };
    out.push(kv("Protocol", protocol));
    out.push(kv("Addressing", if is_29bit(&v.protocol) { "29-bit".into() } else { "11-bit".into() }));
    let cals: Vec<String> = v.ecus.iter().flat_map(|e| e.calibration_ids.iter().cloned()).collect();
    out.push(kv("Calibration", if cals.is_empty() { "—".into() } else { cals.join(" · ") }));
    out.push(Line::raw(""));

    out.push(Line::from(Span::styled(format!("ECUs ({})", v.ecus.len()), bold)));
    out.push(Line::from(Span::styled(format!("  {:<8}{:<26}{}", "ID", "NAME", "CALIBRATION"), dim)));
    if v.ecus.is_empty() {
        out.push(Line::from(Span::styled("  (none answered)", dim)));
    }
    for e in &v.ecus {
        let (name, style) = if !e.name.trim().is_empty() {
            (e.name.clone(), Style::default())
        } else if let Some(n) = conventional_name(&e.controller_id) {
            (n.to_string(), dim)
        } else {
            (format!("ECU {}", e.controller_id), dim)
        };
        out.push(Line::from(vec![
            Span::raw(format!("  {:<8}", e.controller_id)),
            Span::styled(format!("{:<26}", name), style),
            Span::styled(e.calibration_ids.join(" · "), dim),
        ]));
    }
    out.push(Line::raw(""));

    // Supported PIDs grouped by mode, as a grid sized to the width.
    let per_row = ((width.saturating_sub(4)) / 6).max(1) as usize;
    for mode in ["01", "09", "06"] {
        let codes: Vec<&String> = v.supported_pids.iter().filter(|p| p.starts_with(mode)).collect();
        if codes.is_empty() {
            continue;
        }
        out.push(Line::from(Span::styled(format!("SUPPORTED PIDs · mode {mode} ({})", codes.len()), bold)));
        for chunk in codes.chunks(per_row) {
            let row: Vec<String> = chunk.iter().map(|c| format!("{:<6}", c)).collect();
            out.push(Line::raw(format!("  {}", row.concat())));
        }
        out.push(Line::raw(""));
    }
    out
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let Some(v) = app.vehicle.as_deref() else { return };
    let all = lines(v, area.width);
    let max_scroll = (all.len() as u16).saturating_sub(area.height);
    let scroll = app.scroll.min(max_scroll);
    f.render_widget(Paragraph::new(all).scroll((scroll, 0)), area);
}

/// Number of lines the screen would render (test helper for the reflow check).
#[cfg(test)]
pub fn line_count(v: &VehicleInfo, width: u16) -> usize {
    lines(v, width).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use obd_session_rs::vehicle_info::ECUInfo;

    fn ecu(id: &str, name: &str, cal: &str) -> ECUInfo {
        ECUInfo {
            controller_id: id.into(),
            name: name.into(),
            calibration_ids: if cal.is_empty() { vec![] } else { vec![cal.into()] },
            hardware: None,
            strategy: None,
            serial: None,
            module_vin: None,
        }
    }

    pub fn fixture(bits29: bool, named: bool) -> VehicleInfo {
        let (a, b, vin, proto) = if bits29 {
            ("10", "18", "OMA29BIT000000002", "ISO 15765-4 CAN 29-bit (500k)")
        } else {
            ("7E0", "7E1", "OMA11BIT000000001", "ISO 15765-4 CAN 11-bit (500k)")
        };
        VehicleInfo {
            vin: vin.into(),
            wmi: "OMA".into(),
            manufacturer: None,
            engine_type: EngineType::Gasoline,
            ecus: vec![
                ecu(a, if named { "ECM-EngineControl" } else { "" }, "OMATACH-ECM-CAL01"),
                ecu(b, if named { "TCM-TransmisCtrl" } else { "" }, "OMATACH-TCM-CAL01"),
                ecu("7E2", "", ""),
            ],
            supported_pids: (1..=42).map(|i| format!("01{i:02X}")).chain(["0902".to_string(), "0904".to_string(), "090A".to_string()]).collect(),
            protocol: proto.into(),
            year_char: Some("T".into()),
            pcm_strategy: None,
            module_census: vec![],
            strategy_probed: None,
        }
    }

    fn text(lines: Vec<Line<'static>>) -> String {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn eleven_bit_with_names_and_calibrations() {
        let s = text(lines(&fixture(false, true), 80));
        assert!(s.contains("VIN           OMA11BIT000000001"), "{s}");
        assert!(s.contains("Addressing    11-bit"), "{s}");
        assert!(s.contains("ECUs (3)"), "{s}");
        assert!(s.contains("7E0     ECM-EngineControl") && s.contains("OMATACH-ECM-CAL01"), "{s}");
        assert!(s.contains("Calibration   OMATACH-ECM-CAL01 · OMATACH-TCM-CAL01"), "{s}");
        assert!(s.contains("SUPPORTED PIDs · mode 01 (42)") && s.contains("mode 09 (3)"), "{s}");
    }

    #[test]
    fn conventional_fallback_when_090a_unanswered() {
        let s = text(lines(&fixture(true, false), 80));
        assert!(s.contains("Addressing    29-bit"), "{s}");
        assert!(s.contains("10      Engine (ECM/PCM)"), "{s}");
        assert!(s.contains("18      Transmission (TCM)"), "{s}");
        assert!(s.contains("7E2     ECU 7E2"), "{s}");
    }

    #[test]
    fn pid_grid_reflows_with_width() {
        let wide = line_count(&fixture(false, true), 120);
        let narrow = line_count(&fixture(false, true), 30);
        assert!(narrow > wide, "narrow={narrow} wide={wide}");
    }
}
