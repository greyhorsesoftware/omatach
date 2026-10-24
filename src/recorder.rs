//! Dashboard datalogger — the SAME wide CSV the RevOBD app's `DashboardRecorder`
//! writes (SCT-LiveLink-shaped, the native format; pinned there by
//! DashboardRecorderFormatTests), so a recording from either tool opens in the
//! other's chart view:
//! - column A = `Time (s)`, elapsed seconds from the first row (`%.3f`);
//! - RPM (`010C…`) pinned to column B and engine load (`0104…`) to C when
//!   recorded; the rest keep dashboard order;
//! - rows are DENSE: every column forward-fills its latest value (empty only
//!   before a signal's first sample); values printed like C's `%g`;
//! - header labels `SHORT (unit)`, quoted only when they contain `,` or `"`.
//! One row per value-update batch. Buffered, flushed about once a second.
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Recorder {
    file: BufWriter<File>,
    pub path: PathBuf,
    columns: Vec<(String, String)>,
    t0: Option<f64>,
    started: Instant,
    last: HashMap<String, f64>,
    pub rows: usize,
    last_flush: Instant,
}

/// What the stop popup shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub path: PathBuf,
    pub rows: usize,
    pub seconds: f64,
    pub columns: usize,
}

/// LiveLink column convention: RPM first, engine load second, the rest in order.
pub fn tuner_ordered(columns: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut rpm = Vec::new();
    let mut load = Vec::new();
    let mut rest = Vec::new();
    for c in columns {
        if c.0.starts_with("010C") {
            rpm.push(c);
        } else if c.0.starts_with("0104") {
            load.push(c);
        } else {
            rest.push(c);
        }
    }
    rpm.into_iter().chain(load).chain(rest).collect()
}

fn escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// C `%g` (precision 6): shortest of fixed/scientific, trailing zeros stripped.
pub fn fmt_g(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if !v.is_finite() {
        return if v.is_nan() { "nan".into() } else if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let exp = v.abs().log10().floor() as i32;
    // %g rounds to 6 significant digits FIRST, which can bump the exponent (999999.5 → 1e+06)
    let rounded = {
        let scale = 10f64.powi(5 - exp);
        (v * scale).round() / scale
    };
    let exp = if rounded == 0.0 { exp } else { rounded.abs().log10().floor() as i32 };
    if exp < -4 || exp >= 6 {
        let s = format!("{:.5e}", v);
        let (mant, e) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = strip_zeros(mant);
        let e: i32 = e.parse().unwrap_or(0);
        format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (5 - exp).max(0) as usize;
        strip_zeros(&format!("{:.*}", decimals, v))
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// `recording_2026-09-11T15-20-33Z.csv` — ISO-8601 UTC with `:` → `-`, like the mac.
fn file_name(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let t = secs % 86_400;
    format!("recording_{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}Z.csv", t / 3600, (t % 3600) / 60, t % 60)
}

/// Howard Hinnant's days → civil date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Recorder {
    /// Open `<dir>/recording_<stamp>.csv` and write the header.
    pub fn start(dir: &Path, columns: Vec<(String, String)>) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let columns = tuner_ordered(columns);
        let path = dir.join(file_name(SystemTime::now()));
        let mut file = BufWriter::new(File::create(&path)?);
        let header: Vec<String> = std::iter::once("Time (s)".to_string()).chain(columns.iter().map(|(_, l)| escape(l))).collect();
        writeln!(file, "{}", header.join(","))?;
        Ok(Self { file, path, columns, t0: None, started: Instant::now(), last: HashMap::new(), rows: 0, last_flush: Instant::now() })
    }

    /// One snapshot row: merge `values` (id → metric value), forward-fill, write.
    pub fn record(&mut self, values: &HashMap<String, f64>, timestamp: f64) {
        if self.t0.is_none() {
            self.t0 = Some(timestamp);
        }
        let elapsed = timestamp - self.t0.unwrap_or(timestamp);
        for (k, v) in values {
            self.last.insert(k.clone(), *v);
        }
        let mut fields = vec![format!("{elapsed:.3}")];
        for (id, _) in &self.columns {
            fields.push(self.last.get(id).map(|v| fmt_g(*v)).unwrap_or_default());
        }
        let _ = writeln!(self.file, "{}", fields.join(","));
        self.rows += 1;
        if self.last_flush.elapsed().as_secs_f64() >= 1.0 {
            let _ = self.file.flush();
            self.last_flush = Instant::now();
        }
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Flush and close; returns the summary for the popup.
    pub fn stop(mut self) -> Summary {
        let _ = self.file.flush();
        Summary { path: self.path.clone(), rows: self.rows, seconds: self.elapsed(), columns: self.columns.len() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_format_matches_c() {
        for (v, want) in [(750.0, "750"), (47.25, "47.25"), (0.005, "0.005"), (16383.75, "16383.8"), (100000.0, "100000"), (1e6, "1e+06"), (0.00001234, "1.234e-05"), (-2.5, "-2.5"), (14.1, "14.1"), (0.0, "0")] {
            assert_eq!(fmt_g(v), want, "{v}");
        }
    }

    #[test]
    fn tuner_order_pins_rpm_and_load() {
        let cols = vec![("0105.1".to_string(), "ECT (°C)".to_string()), ("0104.1".into(), "LOAD (%)".into()), ("010C.1".into(), "RPM (rpm)".into())];
        let ids: Vec<String> = tuner_ordered(cols).into_iter().map(|c| c.0).collect();
        assert_eq!(ids, vec!["010C.1", "0104.1", "0105.1"]);
    }

    #[test]
    fn writes_dense_forward_filled_rows_like_the_mac() {
        let dir = std::env::temp_dir().join(format!("omatach-rec-{}", std::process::id()));
        let mut r = Recorder::start(&dir, vec![("0105.1".into(), "ECT (°C)".into()), ("010C.1".into(), "RPM (rpm)".into()), ("COMP:X, y".into(), "a \"b\", c".into())]).unwrap();
        r.record(&HashMap::from([("010C.1".to_string(), 750.0)]), 100.0);
        r.record(&HashMap::from([("0105.1".to_string(), 91.0)]), 100.25);
        r.record(&HashMap::from([("010C.1".to_string(), 2340.5), ("COMP:X, y".to_string(), 1e6)]), 101.0);
        let s = r.stop();
        assert_eq!(s.rows, 3);
        let text = std::fs::read_to_string(&s.path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Time (s),RPM (rpm),ECT (°C),\"a \"\"b\"\", c\"");
        assert_eq!(lines[1], "0.000,750,,");
        assert_eq!(lines[2], "0.250,750,91,");
        assert_eq!(lines[3], "1.000,2340.5,91,1e+06");
        assert!(s.path.file_name().unwrap().to_string_lossy().starts_with("recording_20"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn civil_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_707), (2026, 9, 11));
    }
}
