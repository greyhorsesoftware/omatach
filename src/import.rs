//! Torque CSV import: `~/.config/omatach/pids/*.csv`, scanned at
//! launch and on `r`. A port of the mac app's `PIDImport` / `importExtended`
//! rules: column aliases, BOM/quote-tolerant headers, `0x`-prefixed codes,
//! `(code, header)` grouping into `code.N` datapoints, virtual rows (no
//! ModeAndPID + an equation) as `COMP:<Name>` computed entries with bracket
//! refs rewritten to `val{}`, and every equation compile-checked on import so
//! a bad one is skipped with a logged reason, not a crash.
use std::collections::HashMap;
use std::path::Path;

use obd_equation_rs::ExpressionEvaluator;

use crate::catalog::Entry;

pub const SOURCE: &str = "torque";

#[derive(Debug, Default)]
pub struct ImportReport {
    pub files: usize,
    pub entries: Vec<Entry>,
    pub computed: usize,
    /// "file: row N: reason"
    pub skipped: Vec<String>,
    /// Comment / credit lines (a name, no code, no equation).
    pub ignored: usize,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        if self.files == 0 {
            return "pids/: no CSV files".into();
        }
        let mut s = format!("pids/: {} file{}, {} PID{}", self.files, plural(self.files), self.entries.len(), plural(self.entries.len()));
        if self.computed > 0 {
            s.push_str(&format!(" ({} computed)", self.computed));
        }
        if !self.skipped.is_empty() {
            s.push_str(&format!(", {} row{} skipped (see log)", self.skipped.len(), plural(self.skipped.len())));
        }
        s
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Every `*.csv` in `dir`, sorted by name.
pub fn scan_dir(dir: &Path) -> ImportReport {
    let mut report = ImportReport::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return report };
    let mut files: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case("csv")))
        .collect();
    files.sort();
    for f in files {
        let label = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read_to_string(&f) {
            Ok(text) => {
                report.files += 1;
                import_text(&text, &label, &mut report);
            }
            Err(e) => report.skipped.push(format!("{label}: unreadable: {e}")),
        }
    }
    report
}

/// Import one CSV's text into `report`.
pub fn import_text(text: &str, file: &str, report: &mut ImportReport) {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else { return };
    let mut col: HashMap<String, usize> = HashMap::new();
    for (i, raw) in header.iter().enumerate() {
        col.entry(column_key(raw)).or_insert(i);
    }
    let field = |row: &[String], names: &[&str]| -> Option<String> {
        for n in names {
            if let Some(&i) = col.get(*n) {
                if let Some(v) = row.get(i) {
                    let v = v.trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        None
    };
    let missing: Vec<&str> = ["Name", "ModeAndPID", "Equation"].into_iter().filter(|n| !col.contains_key(*n)).collect();
    if !missing.is_empty() {
        report.skipped.push(format!("{file}: missing column(s) {}", missing.join(", ")));
        return;
    }

    // (code, header) → rows, first-seen order
    let mut order: Vec<(String, Option<String>)> = Vec::new();
    let mut groups: HashMap<(String, Option<String>), Vec<(usize, Vec<String>)>> = HashMap::new();
    let mut evaluator = ExpressionEvaluator::new().expect("evaluator");

    for (n, row) in rows.iter().enumerate().skip(1) {
        let line = n + 1;
        let name = field(row, &["Name"]).unwrap_or_default();
        let raw_code = field(row, &["ModeAndPID"]).unwrap_or_default();
        let equation = field(row, &["Equation"]);
        let skip = |report: &mut ImportReport, why: &str| report.skipped.push(format!("{file}: row {line}: {why}"));

        if raw_code.is_empty() {
            let Some(eq) = equation else {
                if !name.is_empty() {
                    report.ignored += 1;
                }
                continue;
            };
            if name.is_empty() {
                skip(report, "computed pid needs a name");
                continue;
            }
            let eq = rewrite_bracket_refs(&eq);
            if !computed_parses(&mut evaluator, &eq) {
                skip(report, &format!("equation didn't parse: {eq}"));
                continue;
            }
            let id = format!("COMP:{name}");
            report.entries.push(Entry {
                id: id.clone(),
                code: id,
                header: None,
                name: name.clone(),
                short_name: field(row, &["ShortName"]),
                unit: field(row, &["Units", "Unit"]).unwrap_or_default(),
                equation: Some(eq),
                min: field(row, &["Min Value", "Min"]).and_then(|v| v.parse().ok()),
                max: field(row, &["Max Value", "Max"]).and_then(|v| v.parse().ok()),
                category: Some("Computed".into()),
                enable_test: None,
                source: SOURCE.into(),
                computed: true,
            });
            report.computed += 1;
            continue;
        }
        let Some(code) = normalized_code(&raw_code) else {
            skip(report, &format!("invalid PID code {raw_code:?}"));
            continue;
        };
        if name.is_empty() {
            skip(report, "missing name");
            continue;
        }
        let Some(eq) = equation else {
            skip(report, "missing equation");
            continue;
        };
        if !wire_parses(&mut evaluator, &eq) {
            skip(report, &format!("equation didn't parse: {eq}"));
            continue;
        }
        let header = field(row, &["OBD Header", "Header", "obdHeader"]).filter(|h| !h.eq_ignore_ascii_case("auto"));
        let header = match header {
            Some(h) if is_hex_header(&h) => Some(h.to_uppercase()),
            Some(h) => {
                skip(report, &format!("header {h:?} is a module name — omatach needs a hex address (7E0 / 18DA10F1)"));
                continue;
            }
            None => None,
        };
        let key = (code.clone(), header.clone());
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push((line, row.clone()));
    }

    for key in order {
        let (code, header) = &key;
        let group = &groups[&key];
        let first = &group[0].1;
        let short = if group.len() == 1 { field(first, &["ShortName"]) } else { None };
        for (i, (_, row)) in group.iter().enumerate() {
            let base = match header {
                Some(h) => format!("{code}@{h}"),
                None => code.clone(),
            };
            report.entries.push(Entry {
                id: format!("{base}.{}", i + 1),
                code: code.clone(),
                header: header.clone(),
                name: field(row, &["Name"]).unwrap_or_else(|| code.clone()),
                short_name: short.clone().or_else(|| field(row, &["ShortName"])),
                unit: field(row, &["Units", "Unit"]).unwrap_or_default(),
                equation: field(row, &["Equation"]),
                min: field(row, &["Min Value", "Min"]).and_then(|v| v.parse().ok()),
                max: field(row, &["Max Value", "Max"]).and_then(|v| v.parse().ok()),
                category: None,
                enable_test: field(row, &["Enable Test", "enableTest", "EnableTest"]),
                source: SOURCE.into(),
                computed: false,
            });
        }
    }
}

/// Header cell → lookup key: strip BOM, surrounding literal quotes, whitespace.
fn column_key(raw: &str) -> String {
    let mut k = raw.replace('\u{FEFF}', "").trim().to_string();
    if k.len() >= 2 && k.starts_with('"') && k.ends_with('"') {
        k = k[1..k.len() - 1].trim().to_string();
    }
    k
}

/// `"0x22dd68"` → `"22DD68"`; 4 or 6 hex chars (or the mode-23 memory forms).
pub fn normalized_code(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.len() >= 2 && s[..2].eq_ignore_ascii_case("0x") {
        s = &s[2..];
    }
    let s = s.to_uppercase();
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if s.len() == 4 || s.len() == 6 || (s.starts_with("2314") && s.len() == 14) || (s.starts_with("2324") && s.len() == 16) {
        Some(s)
    } else {
        None
    }
}

fn is_hex_header(h: &str) -> bool {
    let n = h.len();
    (n == 3 || n == 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

/// `[22436B]` → `val{22436B}` (4–8 hex chars); idempotent on `val{}`-only text.
pub fn rewrite_bracket_refs(eq: &str) -> String {
    let b = eq.as_bytes();
    let mut out = String::with_capacity(eq.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            let n = j - (i + 1);
            if (4..=8).contains(&n) && j < b.len() && b[j] == b']' {
                out.push_str("val{");
                out.push_str(&eq[i + 1..j]);
                out.push('}');
                i = j + 1;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// The `VAL{…}` reference names in an equation, in order, deduplicated (case-insensitive).
pub fn refs(eq: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lower = eq.to_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find("val{") {
        let start = from + p + 4;
        let Some(end_rel) = eq[start..].find('}') else { break };
        let name = eq[start..start + end_rel].trim().to_string();
        if !name.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
            out.push(name);
        }
        from = start + end_rel + 1;
    }
    out
}

fn wire_parses(ev: &mut ExpressionEvaluator, eq: &str) -> bool {
    let bytes = [1u8; 64];
    let vars = HashMap::new();
    ev.evaluate_with_bytes_unified(eq, &vars, &bytes).is_ok()
}

fn computed_parses(ev: &mut ExpressionEvaluator, eq: &str) -> bool {
    let bytes = [1u8; 64];
    let vars = HashMap::new();
    let pids: HashMap<String, f64> = refs(eq).into_iter().map(|r| (r, 1.0)).collect();
    ev.evaluate_with_bytes_and_pids_unified(eq, &vars, &bytes, &pids).is_ok()
}

/// RFC-4180-ish CSV: quotes, `""` escapes, CRLF/CR/LF line endings.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    if row.iter().any(|f| !f.trim().is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{FEFF}Name,ShortName,ModeAndPID,Equation,Min Value,Max Value,Units,Header,startDiagnostic,stopDiagnostic,Scale,Description\r\n\
ABS Wheel Speed 1,ABS RFW,0x222b06,A,0,200,km/h,7E0,,,1\r\n\
\"Trans Temp, oil\",TFT,0x221E1C,\"A-40\",-40,215,°C,auto,,,1,\"has, comma\"\r\n\
Bad Header,BH,0x222B07,A,0,200,km/h,ABS,,,1\r\n\
Broken Eq,BE,0x220100,A +* B,0,1,,7E0,,,1\r\n\
Not Hex,NH,0xZZ12,A,0,1,,,,,1\r\n\
Double RPM,DRPM,,VAL{RPM}*2,0,20000,rpm,,,,1\r\n\
Bolt Power,BPWR,,[22436B]*val{Battery Voltage}/1000,0,200,kW,,,,1\r\n\
Coolant F,ECTF,0x0105,A*1.8-40,-40,420,°F,,,,1\r\n\
credits: someone,,,,,,,,,,\r\n";

    #[test]
    fn csv_parser_handles_quotes_bom_and_crlf() {
        let rows = parse_csv(SAMPLE);
        assert_eq!(rows[0][0], "\u{FEFF}Name");
        assert_eq!(rows[2][0], "Trans Temp, oil");
        assert_eq!(rows[2][11], "has, comma");
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn import_rules_match_the_mac_importer() {
        let mut r = ImportReport::default();
        import_text(SAMPLE, "test.csv", &mut r);
        let ids: Vec<&str> = r.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["COMP:Double RPM", "COMP:Bolt Power", "222B06@7E0.1", "221E1C.1", "0105.1"], "{ids:?}");
        let abs = &r.entries[2];
        assert_eq!((abs.code.as_str(), abs.header.as_deref(), abs.unit.as_str(), abs.max), ("222B06", Some("7E0"), "km/h", Some(200.0)));
        assert_eq!(r.entries[3].header, None, "auto → no header");
        assert_eq!(r.entries[3].name, "Trans Temp, oil");
        let bolt = &r.entries[1];
        assert!(bolt.computed && bolt.equation.as_deref() == Some("val{22436B}*val{Battery Voltage}/1000"), "{bolt:?}");
        assert_eq!(r.computed, 2);
        assert_eq!(r.ignored, 1, "credits line");
        assert_eq!(r.skipped.len(), 3, "{:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.contains("row 4") && s.contains("module name")), "{:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.contains("row 5") && s.contains("didn't parse")), "{:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.contains("row 6") && s.contains("invalid PID code")), "{:?}", r.skipped);
        assert!(r.entries.iter().all(|e| e.source == "torque"));
    }

    #[test]
    fn refs_and_bracket_rewrite() {
        assert_eq!(refs("VAL{RPM} * val{ Speed } + VAL{rpm}"), vec!["RPM", "Speed"]);
        assert_eq!(rewrite_bracket_refs("[22436B]*2 + [12]"), "val{22436B}*2 + [12]");
        assert_eq!(normalized_code("0x22dd68").as_deref(), Some("22DD68"));
        assert_eq!(normalized_code("010c").as_deref(), Some("010C"));
        assert_eq!(normalized_code("22"), None);
    }

    #[test]
    fn scan_dir_reads_every_csv_sorted() {
        let tmp = std::env::temp_dir().join(format!("omatach-pids-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("b.csv"), "Name,ModeAndPID,Equation\nB1,0x010D,A\n").unwrap();
        std::fs::write(tmp.join("a.CSV"), "Name,ModeAndPID,Equation\nA1,0x010C,A\n").unwrap();
        std::fs::write(tmp.join("notes.txt"), "ignored").unwrap();
        let r = scan_dir(&tmp);
        assert_eq!(r.files, 2);
        assert_eq!(r.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["A1", "B1"]);
        assert!(r.summary().starts_with("pids/: 2 files, 2 PIDs"), "{}", r.summary());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
