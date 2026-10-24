//! The Support modal.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::support::{qr_lines, SUPPORT_LABEL, SUPPORT_URL};

pub fn draw(f: &mut Frame, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let qr = qr_lines(SUPPORT_URL);
    let qr_w = qr.first().map(|l| l.chars().count()).unwrap_or(0) as u16;
    let text_w: u16 = (SUPPORT_LABEL.chars().count() as u16 + 1).max(36);
    let w = (qr_w + text_w + 6).min(area.width).max(30);
    let h = ((qr.len() as u16).max(9) + 4).min(area.height);
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, rect);
    let block = Block::bordered().border_style(dim).title(Span::styled(" Support omatach ", Style::default().add_modifier(Modifier::BOLD)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // QR on the left (if it fits), text on the right
    let show_qr = inner.width >= qr_w + text_w + 4;
    let mut lines: Vec<Line> = Vec::new();
    let text: Vec<Line> = vec![
        Line::from(Span::styled("omatach is free & donationware —", Style::default().add_modifier(Modifier::BOLD))),
        Line::raw("if it saved you a trip to the shop,"),
        Line::raw("consider chipping in ♥  (scan or press o)"),
        Line::raw(""),
        Line::from(Span::styled(SUPPORT_LABEL, Style::default().fg(Color::Cyan))),
        Line::raw(""),
        Line::from(vec![Span::styled("o", Style::default().add_modifier(Modifier::BOLD)), Span::styled("  open PayPal in the browser", dim)]),
        Line::from(vec![Span::styled("esc", Style::default().add_modifier(Modifier::BOLD)), Span::styled("  close", dim)]),
    ];
    let rows = if show_qr { qr.len().max(text.len()) } else { text.len() };
    for i in 0..rows {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        if show_qr {
            let q = qr.get(i).cloned().unwrap_or_else(|| " ".repeat(qr_w as usize));
            spans.push(Span::raw(q));
            spans.push(Span::raw("   "));
        }
        if let Some(t) = text.get(i) {
            spans.extend(t.spans.iter().cloned());
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner });
}
