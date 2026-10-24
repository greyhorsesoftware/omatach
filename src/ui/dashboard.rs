//! Dashboard: a responsive grid of gauge cells in three built-in
//! styles — `dial` (braille half-arc with ticks, needle, redline), `bar`
//! (block meter with zones + a sparkline of recent history), `digits` (big
//! box-drawing numerals) — plus the `+` PID picker. ANSI-16 only.
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CLine, Points};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::catalog::Value;
use crate::dash::GaugeStyle;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    if app.dash.cells.is_empty() {
        let msg = if app.is_connected() { "No gauges yet — press + to add PIDs." } else { "Not connected — press c to pick an adapter." };
        f.render_widget(Paragraph::new(Line::from(Span::styled(msg, dim))), pad(area, 2, 1));
        return;
    }
    let n = app.dash.cells.len() as u16;
    let cell_w_min: u16 = 26;
    let cols = (area.width / cell_w_min).clamp(1, n.max(1));
    let cell_w = area.width / cols;
    // each row is as tall as its tallest gauge: dials 10, everything else 6
    let is_tall = |c: &crate::dash::Cell| c.style == GaugeStyle::Dial && app.units.braille();
    let mut y = area.y;
    for (r, row) in app.dash.cells.chunks(cols as usize).enumerate() {
        let want: u16 = if row.iter().any(is_tall) { 10 } else { 6 };
        let row_h = want.min(area.y + area.height - y);
        if row_h < 4 {
            break; // clipped — more rows than the tile fits
        }
        for (c, cell) in row.iter().enumerate() {
            let i = r * cols as usize + c;
            let rect = Rect { x: area.x + c as u16 * cell_w, y, width: cell_w, height: row_h };
            draw_cell(f, app, i, &cell.id, cell.style, rect);
        }
        y += row_h;
    }
}

fn draw_cell(f: &mut Frame, app: &App, idx: usize, id: &str, style: GaugeStyle, rect: Rect) {
    let Some(entry) = app.catalog.get(id) else { return };
    let selected = idx == app.dash.selected && app.screen == crate::app::Screen::Dashboard;
    let inner = pad(rect, 1, 0);
    if inner.height < 2 {
        return;
    }
    let sample = app.dash.latest.get(id);
    let (value_text, num, unit) = match sample.map(|s| &s.value) {
        Some(Value::Num(v)) => {
            let (dv, du) = app.units.display(*v, &entry.unit);
            (fmt_value(dv), Some(*v), du)
        }
        Some(Value::Text(t)) => (t.clone(), None, String::new()),
        None => ("—".into(), None, app.units.display(0.0, &entry.unit).1),
    };
    let (lo, hi) = range(app, id, entry.min, entry.max);
    let frac = num.map(|v| ((v - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0));
    let zone = zone_color(frac);

    // title line
    let title_style = if selected { Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
    let label = trunc(entry.label(), inner.width.saturating_sub(unit.chars().count() as u16 + 1) as usize);
    let pad_n = inner.width.saturating_sub((label.chars().count() + unit.chars().count()) as u16) as usize;
    let title = Line::from(vec![
        Span::styled(label, title_style),
        Span::raw(" ".repeat(pad_n)),
        Span::styled(unit.clone(), Style::default().fg(Color::DarkGray)),
    ]);
    let [title_a, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    f.render_widget(Paragraph::new(title), title_a);
    if body.height == 0 {
        return;
    }
    match style {
        GaugeStyle::Dial if body.height >= 4 && app.units.braille() => draw_dial(f, body, frac, &value_text, zone),
        GaugeStyle::Digits if body.height >= 3 => draw_digits(f, body, &value_text, zone),
        _ => draw_bar(f, app, id, body, frac, &value_text, zone, lo, hi),
    }
}

fn draw_bar(f: &mut Frame, app: &App, id: &str, body: Rect, frac: Option<f64>, value: &str, zone: Color, lo: f64, hi: f64) {
    let [meter_a, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
    let w = meter_a.width.saturating_sub(value.chars().count() as u16 + 3) as usize;
    // fractional fill: whole cells + one eighth-block cell, so the bar moves in 1/8-cell steps
    let eighths = frac.map(|fr| (fr * w as f64 * 8.0).round() as usize).unwrap_or(0).min(w * 8);
    let (full, part) = (eighths / 8, eighths % 8);
    const EDGE: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
    let mut spans = vec![Span::styled("█".repeat(full), Style::default().fg(zone))];
    let mut used = full;
    if part > 0 && used < w {
        spans.push(Span::styled(EDGE[part].to_string(), Style::default().fg(zone)));
        used += 1;
    }
    spans.push(Span::styled("░".repeat(w.saturating_sub(used)), Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(value.to_string(), Style::default().add_modifier(Modifier::BOLD)));
    f.render_widget(Paragraph::new(Line::from(spans)), meter_a);
    if rest.height == 0 {
        return;
    }
    // The sparkline scales to the OBSERVED range (history min..max) — a MAF at 12
    // of 655 g/s would draw flat on the catalog range — with a floor on the span.
    // one blank row between the meter and the pills, when there is room
    let rest = if rest.height >= 3 { Rect { y: rest.y + 1, height: rest.height - 1, ..rest } } else { rest };
    let spark_h = rest.height.min(3);
    let spark_a = Rect { height: spark_h, ..rest };
    // one pill per two columns; each = (height 0..1 on the observed range, zone colour on the gauge range)
    let slots = (spark_a.width as usize + 1) / 2;
    let pills: Vec<(f64, Color)> = app
        .dash
        .history
        .get(id)
        .map(|h| {
            let take = h.len().min(slots);
            let win: Vec<f64> = h.iter().skip(h.len() - take).copied().collect();
            let (mn, mx) = win.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            // floor the span so a flat signal draws flat instead of amplifying jitter:
            // at least 5% of the gauge range, or 4% of the value when there is no range
            let floor = if hi > lo { (hi - lo) * 0.05 } else { mx.abs().max(1.0) * 0.04 };
            let span = (mx - mn).max(floor);
            let base = mn - (span - (mx - mn)) / 2.0;
            win.iter()
                .map(|v| {
                    let height = ((v - base) / span).clamp(0.0, 1.0);
                    let zone = zone_color(Some(((v - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0)));
                    (height, zone)
                })
                .collect()
        })
        .unwrap_or_default();
    draw_pills(f, spark_a, &pills);
}

/// Equaliser-style pills: one full-cell bar per two columns, eighth-block
/// height steps (8 per row), each bar in its own zone colour. Every sample
/// shows at least one eighth so a flat signal is a clean row of stubs.
fn draw_pills(f: &mut Frame, area: Rect, pills: &[(f64, Color)]) {
    if area.height == 0 || pills.is_empty() {
        return;
    }
    const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let rows = area.height as usize;
    let steps = (rows * 8) as f64;
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for row in 0..rows {
        let row_base = ((rows - 1 - row) * 8) as i64; // bottom row first in level space
        let mut spans: Vec<Span> = Vec::with_capacity(pills.len() * 2);
        for (i, (height, color)) in pills.iter().enumerate() {
            let level = ((height * (steps - 1.0)).round() as i64 + 1).clamp(1, steps as i64);
            let n = (level - row_base).clamp(0, 8) as usize;
            spans.push(Span::styled(EIGHTHS[n].to_string(), Style::default().fg(*color)));
            if i + 1 < pills.len() {
                spans.push(Span::raw(" "));
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_dial(f: &mut Frame, body: Rect, frac: Option<f64>, value: &str, zone: Color) {
    let [canvas_a, value_a] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(body);
    let fr = frac.unwrap_or(0.0);
    // Fill the band between two radii with points sampled finer than the braille
    // dot pitch, so EVERY dot inside it lights: a solid band in any terminal font
    // (a single stroke reads as scattered dots where fonts space the dots out).
    let (w, h) = (canvas_a.width.max(1) as f64, canvas_a.height.max(1) as f64);
    let (dx, dy) = (2.4 / (w * 2.0) / 2.0, 1.2 / (h * 4.0) / 2.0);
    let (r_in, r_out) = (0.84, 1.0);
    let mut lit: Vec<(f64, f64)> = Vec::new();
    let mut dim: Vec<(f64, f64)> = Vec::new();
    let mut red: Vec<(f64, f64)> = Vec::new();
    let mut y: f64 = 0.0;
    while y <= 1.08 {
        let mut x: f64 = -1.2;
        while x <= 1.2 {
            let r = (x * x + y * y).sqrt();
            if r >= r_in && r <= r_out {
                // fraction along the arc: 0 at the left end, 1 at the right
                let t = 1.0 - y.atan2(x) / std::f64::consts::PI;
                // tick gaps at the quarters
                let near_tick = [0.25f64, 0.5, 0.75].iter().any(|k| (t - k).abs() < 0.008);
                if !near_tick {
                    if t < fr {
                        lit.push((x, y));
                    } else if t >= 0.9 {
                        red.push((x, y));
                    } else {
                        dim.push((x, y));
                    }
                }
            }
            x += dx;
        }
        y += dy;
    }
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([-1.2, 1.2])
        .y_bounds([-0.12, 1.08])
        .paint(move |ctx| {
            ctx.draw(&Points { coords: &dim, color: Color::DarkGray });
            ctx.draw(&Points { coords: &red, color: Color::Red });
            ctx.draw(&Points { coords: &lit, color: zone });
            // needle from the hub to just inside the band, plus a hub
            let (nx, ny) = polar(fr, r_in - 0.06);
            ctx.draw(&CLine { x1: 0.0, y1: 0.0, x2: nx, y2: ny, color: Color::White });
            ctx.draw(&CLine { x1: -0.05, y1: 0.0, x2: 0.05, y2: 0.0, color: Color::White });
        });
    f.render_widget(canvas, canvas_a);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(value.to_string(), Style::default().add_modifier(Modifier::BOLD)))).alignment(Alignment::Center),
        value_a,
    );
}

/// Point on the half-arc at fraction `t` (0 = left, 1 = right), radius `r`.
fn polar(t: f64, r: f64) -> (f64, f64) {
    let a = std::f64::consts::PI * (1.0 - t.clamp(0.0, 1.0));
    (r * a.cos(), r * a.sin())
}

fn draw_digits(f: &mut Frame, body: Rect, value: &str, zone: Color) {
    let rows = big_digits(value);
    let width = rows[0].chars().count() as u16;
    let x = body.x + body.width.saturating_sub(width) / 2;
    let y = body.y + body.height.saturating_sub(3) / 2;
    for (i, r) in rows.iter().enumerate() {
        let rect = Rect { x, y: y + i as u16, width: width.min(body.width), height: 1 };
        f.render_widget(Paragraph::new(Line::from(Span::styled(r.clone(), Style::default().fg(zone).add_modifier(Modifier::BOLD)))), rect);
    }
}

/// 3-row box-drawing numerals.
pub fn big_digits(s: &str) -> [String; 3] {
    let glyph = |c: char| -> [&'static str; 3] {
        match c {
            '0' => ["┌─┐", "│ │", "└─┘"],
            '1' => [" ╷ ", " │ ", " ╵ "],
            '2' => ["╶─┐", "┌─┘", "└─╴"],
            '3' => ["╶─┐", "╶─┤", "╶─┘"],
            '4' => ["╷ ╷", "└─┤", "  ╵"],
            '5' => ["┌─╴", "└─┐", "╶─┘"],
            '6' => ["┌─╴", "├─┐", "└─┘"],
            '7' => ["╶─┐", "  │", "  ╵"],
            '8' => ["┌─┐", "├─┤", "└─┘"],
            '9' => ["┌─┐", "└─┤", "╶─┘"],
            '.' => [" ", " ", "▪"],
            '-' => ["   ", "╶─╴", "   "],
            ' ' => [" ", " ", " "],
            _ => ["?", "?", "?"],
        }
    };
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, c) in s.chars().enumerate() {
        let g = glyph(c);
        for r in 0..3 {
            if i > 0 {
                rows[r].push(' ');
            }
            rows[r].push_str(g[r]);
        }
    }
    rows
}

fn range(app: &App, id: &str, min: Option<f64>, max: Option<f64>) -> (f64, f64) {
    match (min, max) {
        (Some(lo), Some(hi)) if hi > lo => (lo, hi),
        _ => match app.dash.observed.get(id) {
            Some((lo, hi)) if hi > lo => (*lo, *hi),
            Some((lo, _)) => (*lo, lo + 1.0),
            None => (0.0, 1.0),
        },
    }
}

fn zone_color(frac: Option<f64>) -> Color {
    match frac {
        Some(f) if f >= 0.9 => Color::Red,
        Some(f) if f >= 0.75 => Color::Yellow,
        Some(_) => Color::Green,
        None => Color::DarkGray,
    }
}

pub fn fmt_value(v: f64) -> String {
    if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else if v.abs() >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

// ---- PID picker overlay ---------------------------------------------------

pub fn draw_picker(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let p = &app.picker;
    let rows = app.picker_rows();
    let w = area.width.min(88).max(20);
    let h = area.height.min((rows.len() as u16 + 6).max(8));
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, rect);
    let block = Block::bordered().border_style(dim).title(Span::styled(" Add PIDs ", Style::default().add_modifier(Modifier::BOLD)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    let scope = if p.only_supported { "‹supported› · all" } else { "supported · ‹all›" };
    lines.push(Line::from(vec![
        Span::styled(" find  ", dim),
        Span::raw(p.input.clone()),
        Span::styled("▏", dim),
        Span::raw(" ".repeat(inner.width.saturating_sub((p.input.chars().count() + 9 + scope.chars().count()) as u16) as usize)),
        Span::styled(scope, dim),
    ]));
    lines.push(Line::from(Span::styled(format!("      {:<7}{:<34}{:<12}{}", "PID", "NAME", "VALUE", "SOURCE"), dim)));
    let list_h = inner.height.saturating_sub(4) as usize;
    let start = p.cursor.saturating_sub(list_h.saturating_sub(1));
    for (i, r) in rows.iter().enumerate().skip(start).take(list_h) {
        let mark = if r.on { "✓" } else { " " };
        let cursor = if i == p.cursor { "▸" } else { " " };
        let style = if i == p.cursor {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if !r.supported {
            dim
        } else {
            Style::default()
        };
        let text = format!(" {cursor} {mark}  {:<7}{:<34}{:<12}{}", r.code, trunc(&r.name, 33), r.value, r.source);
        lines.push(Line::from(Span::styled(trunc(&text, inner.width as usize), style)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!(" ✓ on dashboard ({}) · {} shown · unsupported dim", app.dash.cells.len(), rows.len()),
        dim,
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

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
    use crate::app::{App, Screen};
    use crate::catalog::Value;
    use crate::dash::{Cell, GaugeStyle, Sample};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A connected app with a populated dashboard (no engine).
    fn demo_app() -> App {
        let mut app = App::new(None);
        app.conn = crate::app::ConnState::Connected;
        app.screen = Screen::Dashboard;
        let supported: Vec<String> = ["010C", "010D", "0105", "0111", "010F", "0110", "0142", "0106"].iter().map(|s| s.to_string()).collect();
        let mut vi = obd_session_rs::vehicle_info::VehicleInfo {
            vin: "OMA11BIT000000001".into(), wmi: "OMA".into(), manufacturer: None,
            engine_type: obd_session_rs::vehicle_info::EngineType::Gasoline, ecus: vec![],
            supported_pids: supported.clone(), protocol: "ISO 15765-4 CAN 11-bit (500k)".into(),
            year_char: None, pcm_strategy: None, module_census: vec![], strategy_probed: None,
        };
        vi.supported_pids = supported.clone();
        let _ = app.dash.on_vehicle(&vi.vin, &supported, &app.catalog);
        app.vehicle = Some(Box::new(vi));
        app.dash.cells = vec![
            Cell { id: "010C.1".into(), style: GaugeStyle::Dial },
            Cell { id: "010D.1".into(), style: GaugeStyle::Digits },
            Cell { id: "0105.1".into(), style: GaugeStyle::Bar },
            Cell { id: "0111.1".into(), style: GaugeStyle::Bar },
            Cell { id: "0110.1".into(), style: GaugeStyle::Bar },
            Cell { id: "0142.1".into(), style: GaugeStyle::Bar },
        ];
        let vals: [(&str, f64); 6] = [("010C.1", 2340.0), ("010D.1", 47.0), ("0105.1", 91.0), ("0111.1", 18.0), ("0110.1", 12.4), ("0142.1", 14.1)];
        for (id, v) in vals {
            for k in 0..24 {
                let jitter = ((k as f64) * 0.7).sin() * 0.08 * v;
                app.dash.history.entry(id.to_string()).or_default().push_back(v + jitter);
                let o = app.dash.observed.entry(id.to_string()).or_insert((v, v));
                o.0 = o.0.min(v + jitter); o.1 = o.1.max(v + jitter);
            }
            app.dash.latest.insert(id.into(), Sample { value: Value::Num(v), ecu: "7E8".into() });
        }
        app
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width { line.push_str(buf[(x, y)].symbol()); }
            out.push_str(line.trim_end()); out.push('\n');
        }
        out
    }

    #[test]
    fn dashboard_renders_all_three_styles_and_reflows() {
        let app = demo_app();
        let s = render(&app, 100, 30);
        assert!(s.contains("RPM") && s.contains("2340"), "{s}");
        assert!(s.contains("░") && s.contains("█"), "bar meter\n{s}");
        assert!(s.contains("┌─┐") || s.contains("╷ ╷"), "digits\n{s}");
        assert!(s.chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)), "braille dial\n{s}");
        for (w, h) in [(60, 20), (40, 12), (26, 8), (10, 4)] {
            let _ = render(&app, w, h);
        }
        // picker
        let mut app = app;
        app.overlay = crate::app::Overlay::Picker;
        app.picker.input = "cool".into();
        let s = render(&app, 100, 30);
        assert!(s.contains("Add PIDs") && s.contains("0105") && s.contains("Coolant"), "{s}");
    }

    /// `cargo test print_dashboard_demo -- --nocapture --ignored` → the mockups.
    #[test]
    #[ignore]
    fn print_dashboard_demo() {
        let app = demo_app();
        println!("\n== dashboard 100x28 ==\n{}", render(&app, 100, 28));
        let mut app = app;
        app.overlay = crate::app::Overlay::Picker;
        app.picker.input = "coolant".into();
        println!("\n== picker 100x28 ==\n{}", render(&app, 100, 28));
    }

    #[test]
    fn braille_off_falls_back_to_bars() {
        let mut app = demo_app();
        app.units.set_braille(false);
        let s = render(&app, 100, 30);
        assert!(!s.chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)), "no braille expected\n{s}");
        assert!(s.contains("RPM") && s.contains("2340") && s.contains("░"), "{s}");
    }

    #[test]
    fn digits_font_rows_align() {
        let rows = big_digits("2340.5");
        let w = rows[0].chars().count();
        assert!(rows.iter().all(|r| r.chars().count() == w), "{rows:?}");
        assert!(rows[1].contains('┌'), "{rows:?}");
    }

    #[test]
    fn value_formatting_scales_precision() {
        assert_eq!(fmt_value(2340.0), "2340");
        assert_eq!(fmt_value(47.25), "47.2");
        assert_eq!(fmt_value(2.345), "2.35");
    }

    #[test]
    fn polar_endpoints() {
        let (x, y) = polar(0.0, 1.0);
        assert!((x + 1.0).abs() < 1e-9 && y.abs() < 1e-9);
        let (x, y) = polar(0.5, 1.0);
        assert!(x.abs() < 1e-9 && (y - 1.0).abs() < 1e-9);
    }
}
