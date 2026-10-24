//! Trouble codes: the whole read flow is omatach-side. `03`/`07`/`0A`
//! replies arrive as `obd_data`; each ECU's data bytes go through the engine's
//! count-byte-aware `parse_dtcs`; descriptions come from the vault's five
//! libraries, chosen by the VIN's WMI (make library first, then J1979, then the
//! rest). `04` clears; a `7F 04 22` refusal is detected on `raw_hex` because
//! the engine strips the first reply byte blindly for mode 04.
use std::collections::HashMap;

use obd_session_rs::trouble_code_parser::parse_dtcs;
use serde_json::Value as Json;

use crate::vault;

#[derive(Debug, Clone, PartialEq)]
pub struct Dtc {
    pub code: String,
    pub ecu: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Stored,
    Pending,
    Permanent,
}

impl Kind {
    pub fn command(self) -> &'static str {
        match self {
            Kind::Stored => "03",
            Kind::Pending => "07",
            Kind::Permanent => "0A",
        }
    }
    pub fn from_command(cmd: &str) -> Option<Kind> {
        match cmd.trim().to_uppercase().as_str() {
            "03" => Some(Kind::Stored),
            "07" => Some(Kind::Pending),
            "0A" => Some(Kind::Permanent),
            _ => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Kind::Stored => "Stored",
            Kind::Pending => "Pending",
            Kind::Permanent => "Permanent",
        }
    }
    pub const ALL: [Kind; 3] = [Kind::Stored, Kind::Pending, Kind::Permanent];
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClearResult {
    Cleared,
    /// `7F 04 <nrc>` from an ECU.
    Refused(u8),
    /// No answer / dropped link.
    Failed,
}

impl ClearResult {
    /// The shipped banner copy (Localizable.strings), ported.
    pub fn text(&self) -> String {
        match self {
            ClearResult::Cleared => "Trouble codes cleared".into(),
            ClearResult::Refused(0x22) => "Clear refused — turn the engine off (key on), then try again".into(),
            ClearResult::Refused(nrc) => format!("Clear refused by the vehicle (NRC 7F {nrc:02X})"),
            ClearResult::Failed => "Clear failed — the vehicle refused or the connection dropped".into(),
        }
    }
    pub fn ok(&self) -> bool {
        matches!(self, ClearResult::Cleared)
    }
}

/// The KOEO confirm copy (troubleCodes.alert.clearMessage), verbatim.
pub const KOEO_TITLE: &str = "Clear Trouble Codes";
pub const KOEO_MESSAGE: &str = "Put the vehicle in Key On, Engine Off — ignition on, engine not running — before clearing. Codes can't be cleared while the engine is running. This resets the check engine light if it is on.";

pub struct Library {
    libs: HashMap<String, HashMap<String, String>>,
}

impl Library {
    pub fn from_vault() -> Self {
        Self { libs: vault::json(vault::names::DTCS_JSON) }
    }

    #[cfg(test)]
    pub fn from_map(libs: HashMap<String, HashMap<String, String>>) -> Self {
        Self { libs }
    }

    /// Library order for a WMI: the make's library first, then J1979, then the rest.
    pub fn order(wmi: &str) -> Vec<&'static str> {
        let w = wmi.to_uppercase();
        let two = w.get(..2).unwrap_or("");
        let make: &[&str] = if matches!(two, "1F" | "2F" | "3F") || matches!(w.as_str(), "WF0" | "MAJ" | "NM0" | "1ZV") {
            &["ford"]
        } else if matches!(two, "1C" | "2C" | "3C" | "1B" | "1D" | "1J" | "2A" | "2B" | "2D" | "3D") || matches!(w.as_str(), "ZAC" | "ZAR" | "ZFA") {
            &["fca"]
        } else if matches!(two, "1G" | "2G" | "3G") || matches!(w.as_str(), "KL1" | "KL4" | "KL7" | "KL8" | "W0L") {
            &["gm_global_b", "gm_global_a"]
        } else {
            &[]
        };
        let mut v: Vec<&'static str> = make.to_vec();
        for l in ["j1979", "ford", "fca", "gm_global_b", "gm_global_a"] {
            if !v.contains(&l) {
                v.push(l);
            }
        }
        v
    }

    /// Description for a 5-char code. GM Global B keys carry a failure-type
    /// suffix (`B000111`), so a prefix match backs up the exact one there.
    pub fn describe(&self, code: &str, wmi: &str) -> Option<String> {
        let code = code.to_uppercase();
        for lib in Self::order(wmi) {
            let Some(m) = self.libs.get(lib) else { continue };
            if let Some(d) = m.get(&code) {
                return Some(d.clone());
            }
            if lib == "gm_global_b" {
                let mut keys: Vec<&String> = m.keys().filter(|k| k.starts_with(&code) && k.len() == 7).collect();
                keys.sort();
                if let Some(k) = keys.first() {
                    return Some(m[*k].clone());
                }
            }
        }
        None
    }
}

/// Parse one `03`/`07`/`0A` reply (`obd_data.parsed`) into codes per ECU,
/// lowest controller first. `None` parsed (NO DATA) = no codes.
pub fn parse_reply(parsed: Option<&Json>, lib: &Library, wmi: &str) -> Vec<Dtc> {
    let mut out = Vec::new();
    let Some(ctrls) = parsed.and_then(|p| p["all_controllers"].as_object()) else { return out };
    let mut keys: Vec<&String> = ctrls.keys().collect();
    keys.sort();
    for ecu in keys {
        let c = &ctrls[ecu];
        let bytes: Vec<u8> = c["data_bytes"].as_array().map(|a| a.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect()).unwrap_or_default();
        if raw_is_negative(c) {
            continue;
        }
        for code in parse_dtcs(&bytes).codes {
            out.push(Dtc { description: lib.describe(&code, wmi), code, ecu: ecu.clone() });
        }
    }
    out
}

fn raw_is_negative(c: &Json) -> bool {
    c["raw_hex"].as_str().map(|r| r.trim().to_uppercase().starts_with("7F")).unwrap_or(false)
}

/// Interpret the `04` reply: any ECU's negative response is a refusal; a
/// positive (or any non-negative answer) is success; nothing = failed.
pub fn clear_result(parsed: Option<&Json>) -> ClearResult {
    let Some(ctrls) = parsed.and_then(|p| p["all_controllers"].as_object()) else { return ClearResult::Failed };
    if ctrls.is_empty() {
        return ClearResult::Failed;
    }
    let mut keys: Vec<&String> = ctrls.keys().collect();
    keys.sort();
    for k in keys {
        let c = &ctrls[k];
        if raw_is_negative(c) {
            let raw: Vec<u8> = c["raw_hex"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .filter_map(|t| u8::from_str_radix(t, 16).ok())
                .collect();
            return ClearResult::Refused(raw.get(2).copied().unwrap_or(0));
        }
    }
    ClearResult::Cleared
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(ctrls: &[(&str, &str, &[u8])]) -> Json {
        let mut m = serde_json::Map::new();
        for (ecu, raw, bytes) in ctrls {
            m.insert(ecu.to_string(), serde_json::json!({ "raw_hex": raw, "data_bytes": bytes, "controller_id": ecu, "is_valid": !bytes.is_empty() }));
        }
        serde_json::json!({ "type": "trouble_codes", "all_controllers": m })
    }

    #[test]
    fn stored_codes_parse_per_ecu_with_descriptions() {
        let lib = Library::from_vault();
        let p = parsed(&[("7E9", "43 01 C4 18", &[0x01, 0xC4, 0x18]), ("7E8", "43 02 03 00 01 71", &[0x02, 0x03, 0x00, 0x01, 0x71])]);
        let codes = parse_reply(Some(&p), &lib, "OMA");
        let list: Vec<(&str, &str)> = codes.iter().map(|d| (d.ecu.as_str(), d.code.as_str())).collect();
        assert_eq!(list, vec![("7E8", "P0300"), ("7E8", "P0171"), ("7E9", "U0418")]);
        assert!(codes[0].description.as_deref().unwrap_or("").to_lowercase().contains("misfire"), "{:?}", codes[0]);
        assert!(codes[1].description.as_deref().unwrap_or("").to_lowercase().contains("lean"), "{:?}", codes[1]);
        assert!(codes[2].description.is_some(), "{:?}", codes[2]);
        assert!(parse_reply(None, &lib, "OMA").is_empty());
    }

    #[test]
    fn make_library_wins_over_generic_then_falls_back() {
        let mut libs = HashMap::new();
        libs.insert("j1979".to_string(), HashMap::from([("P0300".to_string(), "generic".to_string())]));
        libs.insert("ford".to_string(), HashMap::from([("P0300".to_string(), "ford text".to_string()), ("P1234".to_string(), "ford only".to_string())]));
        libs.insert("gm_global_b".to_string(), HashMap::from([("B000111".to_string(), "gm b prefix".to_string())]));
        let lib = Library::from_map(libs);
        assert_eq!(lib.describe("P0300", "1FA").as_deref(), Some("ford text"));
        assert_eq!(lib.describe("P0300", "OMA").as_deref(), Some("generic"));
        assert_eq!(lib.describe("P1234", "OMA").as_deref(), Some("ford only"), "falls back across makes");
        assert_eq!(lib.describe("B0001", "1G1").as_deref(), Some("gm b prefix"));
        assert_eq!(lib.describe("P9999", "1G1"), None);
        assert_eq!(Library::order("1C4")[0], "fca");
    }

    #[test]
    fn clear_result_reads_the_negative_from_raw_hex() {
        assert_eq!(clear_result(Some(&parsed(&[("7E8", "44", &[])]))), ClearResult::Cleared);
        assert_eq!(clear_result(Some(&parsed(&[("7E8", "7F 04 22", &[0x04, 0x22])]))), ClearResult::Refused(0x22));
        assert_eq!(clear_result(Some(&parsed(&[("7E8", "44", &[]), ("7E9", "7F 04 31", &[0x04, 0x31])]))), ClearResult::Refused(0x31));
        assert_eq!(clear_result(None), ClearResult::Failed);
        assert!(ClearResult::Refused(0x22).text().contains("turn the engine off"));
    }
}
