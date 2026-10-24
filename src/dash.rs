//! Dashboard state: which entries are on the dashboard and in what
//! style, their latest values + history, enable-test hiding, computed entries,
//! and per-VIN persistence. Pure data + decode; the engine calls come out of
//! the App as `Action`s.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::catalog::{Catalog, Value};

pub const HISTORY: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GaugeStyle {
    #[default]
    Bar,
    Dial,
    Digits,
}

impl GaugeStyle {
    pub fn next(self) -> Self {
        match self {
            GaugeStyle::Bar => GaugeStyle::Dial,
            GaugeStyle::Dial => GaugeStyle::Digits,
            GaugeStyle::Digits => GaugeStyle::Bar,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub id: String,
    #[serde(default)]
    pub style: GaugeStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DashFile {
    #[serde(default)]
    cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub value: Value,
    pub ecu: String,
}

/// The default layout for a fresh VIN, filtered to what the car supports.
pub const DEFAULT_CELLS: &[&str] = &["010C.1", "010D.1", "0105.1", "0111.1", "010F.1", "0110.1", "0142.1", "0106.1", "0104.1", "010B.1"];

pub struct Dashboard {
    pub cells: Vec<Cell>,
    pub selected: usize,
    pub latest: HashMap<String, Sample>,
    pub history: HashMap<String, VecDeque<f64>>,
    /// Observed (min, max) per entry — the meter range when the catalog has none.
    pub observed: HashMap<String, (f64, f64)>,
    /// Entries hidden by a failed enable test (this vehicle lacks the sensor).
    pub hidden: HashSet<String>,
    /// Codes whose enable-test probe reply is still pending.
    pub pending_tests: HashSet<String>,
    pub tests_total: usize,
    /// Mode-01 codes the vehicle reports supported.
    pub supported: HashSet<String>,
    dash_dir: Option<PathBuf>,
    vin: Option<String>,
}

impl Dashboard {
    pub fn new(config_dir: Option<&Path>) -> Self {
        Self {
            cells: Vec::new(),
            selected: 0,
            latest: HashMap::new(),
            history: HashMap::new(),
            observed: HashMap::new(),
            hidden: HashSet::new(),
            pending_tests: HashSet::new(),
            tests_total: 0,
            supported: HashSet::new(),
            dash_dir: config_dir.map(|d| d.join("dashboards")),
            vin: None,
        }
    }

    // ---- lifecycle --------------------------------------------------------

    /// A vehicle identified: adopt its supported set, restore its layout (or the
    /// default), and return the codes to subscribe + the codes to probe for tests.
    pub fn on_vehicle(&mut self, vin: &str, supported: &[String], catalog: &Catalog) -> (Vec<(String, Option<String>)>, Vec<String>) {
        self.supported = supported.iter().filter(|c| c.starts_with("01")).cloned().collect();
        self.vin = Some(if vin.is_empty() { "default".into() } else { vin.to_string() });
        self.latest.clear();
        self.history.clear();
        self.observed.clear();
        self.hidden.clear();
        self.cells = self.restore().unwrap_or_else(|| {
            DEFAULT_CELLS
                .iter()
                .filter(|id| catalog.get(id).map_or(false, |e| self.supported.contains(&e.code)))
                .map(|id| Cell { id: id.to_string(), style: GaugeStyle::Bar })
                .collect()
        });
        if let Some(first) = self.cells.first_mut() {
            if first.style == GaugeStyle::Bar {
                first.style = GaugeStyle::Dial;
            }
        }
        self.selected = 0;
        let mut probes: Vec<String> = self.supported.iter().filter(|c| catalog.code_has_enable_tests(c)).cloned().collect();
        probes.sort();
        self.pending_tests = probes.iter().cloned().collect();
        self.tests_total = probes.len();
        (self.wire_targets(catalog), probes)
    }

    pub fn on_disconnect(&mut self) {
        self.latest.clear();
        self.history.clear();
        self.pending_tests.clear();
        self.tests_total = 0;
        self.supported.clear();
        self.vin = None;
    }

    /// What the dashboard needs polled, as `(code, header)`: wire entries
    /// directly, computed entries through their real inputs. Computed entries
    /// themselves never reach the wire.
    pub fn wire_targets(&self, catalog: &Catalog) -> Vec<(String, Option<String>)> {
        let mut v: Vec<(String, Option<String>)> = Vec::new();
        for c in &self.cells {
            let Some(e) = catalog.get(&c.id) else { continue };
            let targets = if e.computed { catalog.closure(&e.id).inputs } else { vec![(e.code.clone(), e.header.clone())] };
            for t in targets {
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
        v
    }

    // ---- data -------------------------------------------------------------

    /// A reply for `code` arrived: decode every entry of that code (first
    /// responder = lowest controller id), run pending enable tests, refresh
    /// computed entries. Returns true when a dashboard cell changed.
    pub fn on_reply(&mut self, code: &str, parsed: Option<&Json>, catalog: &mut Catalog) -> bool {
        let Some((ecu, bytes)) = first_responder(parsed) else { return false };
        let entries: Vec<(String, Option<String>)> = catalog.entries_for_code(code).iter().map(|e| (e.id.clone(), e.header.clone())).collect();
        if self.pending_tests.remove(code) {
            for (id, _) in &entries {
                if catalog.enable_test(id, &bytes) == Some(false) {
                    self.hidden.insert(id.clone());
                }
            }
        }
        let mut changed = false;
        for (id, header) in &entries {
            // A headered entry decodes the controller that answers for its
            // header when that ECU is in the reply; otherwise the first responder.
            let (ecu, bytes) = match header.as_deref().and_then(|h| responder_for_header(parsed, h)) {
                Some(x) => x,
                None => (ecu.clone(), bytes.clone()),
            };
            if let Some(v) = catalog.decode(id, &bytes) {
                self.push(id, v, &ecu);
                changed |= self.cells.iter().any(|c| &c.id == id);
            }
        }
        // computed entries over the latest numeric values
        let values = self.numeric_by_ref(catalog);
        let computed: Vec<String> = self.cells.iter().filter(|c| catalog.get(&c.id).map_or(false, |e| e.computed)).map(|c| c.id.clone()).collect();
        for id in computed {
            if let Some(v) = catalog.compute(&id, &values) {
                self.push(&id, v, "");
                changed = true;
            }
        }
        changed
    }

    fn push(&mut self, id: &str, v: Value, ecu: &str) {
        if let Value::Num(n) = v {
            let h = self.history.entry(id.to_string()).or_default();
            h.push_back(n);
            while h.len() > HISTORY {
                h.pop_front();
            }
            let o = self.observed.entry(id.to_string()).or_insert((n, n));
            o.0 = o.0.min(n);
            o.1 = o.1.max(n);
        }
        self.latest.insert(id.to_string(), Sample { value: v, ecu: ecu.to_string() });
    }

    /// Latest numeric values keyed every way a `VAL{…}` ref may spell them:
    /// short name, name, code, id.
    fn numeric_by_ref(&self, catalog: &Catalog) -> HashMap<String, f64> {
        let mut m = HashMap::new();
        for (id, s) in &self.latest {
            let Some(n) = s.value.as_f64() else { continue };
            if let Some(e) = catalog.get(id) {
                if let Some(sn) = &e.short_name {
                    m.insert(sn.clone(), n);
                }
                m.insert(e.name.clone(), n);
                m.insert(e.code.clone(), n);
            }
            m.insert(id.clone(), n);
        }
        m
    }

    // ---- editing ----------------------------------------------------------

    pub fn has(&self, id: &str) -> bool {
        self.cells.iter().any(|c| c.id == id)
    }

    pub fn add(&mut self, id: &str) {
        if !self.has(id) {
            self.cells.push(Cell { id: id.to_string(), style: GaugeStyle::Bar });
            self.persist();
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.cells.retain(|c| c.id != id);
        if self.selected >= self.cells.len() {
            self.selected = self.cells.len().saturating_sub(1);
        }
        self.persist();
    }

    pub fn remove_selected(&mut self) -> Option<String> {
        let id = self.cells.get(self.selected)?.id.clone();
        self.remove(&id);
        Some(id)
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.cells.len() {
            self.selected += 1;
        }
    }

    pub fn move_selected(&mut self, delta: i32) {
        let n = self.cells.len();
        if n < 2 {
            return;
        }
        let to = (self.selected as i32 + delta).clamp(0, n as i32 - 1) as usize;
        if to != self.selected {
            self.cells.swap(self.selected, to);
            self.selected = to;
            self.persist();
        }
    }

    pub fn cycle_style(&mut self) {
        if let Some(c) = self.cells.get_mut(self.selected) {
            c.style = c.style.next();
            self.persist();
        }
    }

    pub fn tests_done(&self) -> usize {
        self.tests_total - self.pending_tests.len()
    }

    // ---- persistence (per VIN) -------------------------------------------

    fn file(&self) -> Option<PathBuf> {
        Some(self.dash_dir.as_ref()?.join(format!("{}.toml", self.vin.as_deref()?)))
    }

    fn persist(&self) {
        let Some(p) = self.file() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string(&DashFile { cells: self.cells.clone() }) {
            let _ = std::fs::write(p, s);
        }
    }

    fn restore(&self) -> Option<Vec<Cell>> {
        let s = std::fs::read_to_string(self.file()?).ok()?;
        let f: DashFile = toml::from_str(&s).ok()?;
        if f.cells.is_empty() { None } else { Some(f.cells) }
    }
}

/// The controller that answers for a request header: `7E0` → `7E8` (11-bit), an
/// 8-hex 29-bit header `18DAxxF1` → source byte `xx`.
pub fn responder_for_header(parsed: Option<&Json>, header: &str) -> Option<(String, Vec<u8>)> {
    let ctrls = parsed?["all_controllers"].as_object()?;
    let h = header.to_uppercase();
    let key = if h.len() == 3 {
        u16::from_str_radix(&h, 16).ok().map(|n| format!("{:03X}", n + 8))?
    } else if h.len() == 8 && h.starts_with("18DA") {
        h[4..6].to_string()
    } else {
        return None;
    };
    let c = ctrls.get(&key)?;
    let bytes: Vec<u8> = c["data_bytes"].as_array()?.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect();
    Some((key, bytes))
}

/// The first responder: the controller with the lowest id (the engine ECU on
/// every layout we know), as `(id, data_bytes)`.
pub fn first_responder(parsed: Option<&Json>) -> Option<(String, Vec<u8>)> {
    let ctrls = parsed?["all_controllers"].as_object()?;
    let mut keys: Vec<&String> = ctrls.keys().collect();
    keys.sort();
    for k in keys {
        let c = &ctrls[k];
        if c["is_valid"].as_bool() == Some(false) {
            continue;
        }
        let bytes: Vec<u8> = c["data_bytes"].as_array()?.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect();
        return Some((k.clone(), bytes));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn parsed(ecu: &str, bytes: &[u8]) -> Json {
        serde_json::json!({ "type": "pid_response", "all_controllers": { ecu: { "raw_hex": "", "data_bytes": bytes, "controller_id": ecu, "is_valid": true } } })
    }

    #[test]
    fn vehicle_defaults_then_reply_updates_and_persists_per_vin() {
        let tmp = std::env::temp_dir().join(format!("omatach-dash-{}", std::process::id()));
        let mut cat = Catalog::from_vault();
        let mut d = Dashboard::new(Some(&tmp));
        let supported: Vec<String> = ["010C", "010D", "0105", "0178"].iter().map(|s| s.to_string()).collect();
        let (codes, probes) = d.on_vehicle("VIN1", &supported, &cat);
        assert_eq!(codes, vec![("010C".to_string(), None), ("010D".to_string(), None), ("0105".to_string(), None)]);
        assert_eq!(probes, vec!["0178"]);
        assert_eq!(d.cells[0].style, GaugeStyle::Dial);
        assert!(d.on_reply("010C", Some(&parsed("7E8", &[0x0B, 0xB8])), &mut cat));
        assert_eq!(d.latest["010C.1"].value, Value::Num(750.0));
        assert_eq!(d.history["010C.1"].len(), 1);
        // enable test: only EGT 1+2 fitted → 0178.3/.4 hidden
        assert!(!d.on_reply("0178", Some(&parsed("7E8", &[0b11, 0, 0, 0, 0, 0, 0, 0, 0])), &mut cat));
        assert!(d.hidden.contains("0178.3") && !d.hidden.contains("0178.1"));
        assert_eq!(d.tests_done(), 1);
        // edit + reorder persist, and come back for the same VIN
        d.add("0111.1");
        d.selected = 3;
        d.move_selected(-1);
        assert_eq!(d.cells[2].id, "0111.1");
        d.cycle_style();
        let mut d2 = Dashboard::new(Some(&tmp));
        let _ = d2.on_vehicle("VIN1", &supported, &cat);
        assert_eq!(d2.cells, d.cells);
        let mut d3 = Dashboard::new(Some(&tmp));
        let _ = d3.on_vehicle("VIN2", &supported, &cat);
        assert_eq!(d3.cells.len(), 3, "other VIN gets the defaults");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn first_responder_is_the_lowest_controller() {
        let j = serde_json::json!({ "all_controllers": {
            "7E9": { "data_bytes": [1], "is_valid": true },
            "7E8": { "data_bytes": [2], "is_valid": true } } });
        assert_eq!(first_responder(Some(&j)), Some(("7E8".into(), vec![2])));
    }
}
