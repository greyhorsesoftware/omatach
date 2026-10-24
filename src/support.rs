//! Support / Donate: omatach is free donationware. One canonical URL
//! (a QR encodes a single link), rendered as a half-block terminal QR so a
//! phone can scan it, plus `o` to open it in the browser.
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

/// The one canonical donation link (decided 2026-09-11: PayPal Donate, hosted
/// button). A QR encodes a single link — this is it.
pub const SUPPORT_URL: &str = "https://www.paypal.com/donate/?hosted_button_id=VRTDT7PVAFARL";
/// What the modal shows instead of the raw URL.
pub const SUPPORT_LABEL: &str = "paypal.com/donate  (Grey Horse Software)";

/// The QR as lines of half-block characters (2 modules per row of text).
pub fn qr_lines(url: &str) -> Vec<String> {
    let Ok(code) = QrCode::new(url.as_bytes()) else { return Vec::new() };
    let s = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(false)
        .build();
    // one-cell light margin of our own (scanners want some, four modules is too wide for a tile)
    let w = s.lines().next().map(|l| l.chars().count()).unwrap_or(0);
    let blank = "█".repeat(w + 2);
    let mut out = vec![blank.clone()];
    out.extend(s.lines().map(|l| format!("█{l}█")));
    out.push(blank);
    out
}

/// Open a URL in the user's browser (`xdg-open` on Linux, `open` on macOS), detached.
pub fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_renders_as_half_blocks() {
        let lines = qr_lines(SUPPORT_URL);
        assert!(lines.len() > 12 && lines.len() < 40, "{}", lines.len());
        let w = lines[0].chars().count();
        assert!(lines.iter().all(|l| l.chars().count() == w), "ragged");
        assert!(lines.iter().any(|l| l.contains('█') || l.contains('▀') || l.contains('▄')));
    }
}
