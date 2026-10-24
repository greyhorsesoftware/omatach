//! The PID catalog: J1979 entries from the vault (`data/j1979_catalog.json`),
//! later Torque-CSV entries. Uniform `code.N` ids. Equations are compiled by
//! `obd_equation_rs` on first evaluation (one evaluator per entry, the crate's
//! own memo) and checked at load so a bad one fails here, not on a gauge.
use std::collections::HashMap;

use obd_equation_rs::{ExpressionEvaluator, ExpressionResult};
use serde::{Deserialize, Serialize};

use crate::vault;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub code: String,
    #[serde(default)]
    pub header: Option<String>,
    pub name: String,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub equation: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub enable_test: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
    /// Computed (Torque "virtual") entry: an equation over other entries via `VAL{…}`
    /// refs, no OBD request of its own. Never reaches the wire.
    #[serde(default)]
    pub computed: bool,
}

fn default_source() -> String {
    "j1979".into()
}

impl Entry {
    /// Label for a gauge: the short name when the PID has one, else the datapoint name.
    pub fn label(&self) -> &str {
        self.short_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Text(String),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Text(_) => None,
        }
    }
}

#[derive(Deserialize)]
struct CatalogFile {
    entries: Vec<Entry>,
}

pub struct Catalog {
    pub entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    by_code: HashMap<String, Vec<usize>>,
    evaluators: HashMap<String, ExpressionEvaluator>,
    /// The built-in (vault) entries, kept so a CSV rescan can rebuild from scratch.
    base: Vec<Entry>,
}

/// A computed entry's dependency closure.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Closure {
    /// Real (pollable) inputs as `(code, header)`, deduped.
    pub inputs: Vec<(String, Option<String>)>,
    /// Refs that resolve to nothing in the catalog.
    pub missing: Vec<String>,
    pub cycle: bool,
}

impl Catalog {
    pub fn from_vault() -> Self {
        let file: CatalogFile = vault::json(vault::names::J1979_CATALOG_JSON);
        Self::from_entries(file.entries)
    }

    pub fn from_entries(entries: Vec<Entry>) -> Self {
        let mut c = Self { entries: Vec::new(), by_id: HashMap::new(), by_code: HashMap::new(), evaluators: HashMap::new(), base: entries.clone() };
        for e in entries {
            c.insert(e);
        }
        c
    }

    /// Rebuild from the built-ins, then layer `extras` (CSV imports) on top —
    /// a same-id extra shadows the built-in; a removed CSV row un-shadows it.
    pub fn reload_extras(&mut self, extras: Vec<Entry>) {
        let base = std::mem::take(&mut self.base);
        *self = Self::from_entries(base);
        for e in extras {
            self.insert(e);
        }
    }

    /// Resolve a `VAL{…}` ref: short name, name, or code (with `0x` stripped),
    /// case-insensitive; later (CSV) entries win over built-ins.
    pub fn find_ref(&self, name: &str) -> Option<&Entry> {
        let key = name.trim().to_lowercase();
        let code_key = key.strip_prefix("0x").unwrap_or(&key).to_string();
        self.entries.iter().rev().find(|e| {
            e.short_name.as_deref().map_or(false, |s| s.to_lowercase() == key)
                || e.name.to_lowercase() == key
                || e.code.to_lowercase() == code_key
                || e.id.to_lowercase() == key
        })
    }

    /// The closure of a computed entry (transitively through other computed entries).
    pub fn closure(&self, id: &str) -> Closure {
        let mut c = Closure::default();
        let mut stack: Vec<String> = Vec::new();
        let mut visited: Vec<String> = Vec::new();
        self.visit(id, &mut c, &mut stack, &mut visited);
        c
    }

    fn visit(&self, id: &str, c: &mut Closure, stack: &mut Vec<String>, visited: &mut Vec<String>) {
        if stack.iter().any(|s| s == id) {
            c.cycle = true;
            return;
        }
        if visited.iter().any(|v| v == id) {
            return;
        }
        let Some(e) = self.get(id) else { return };
        stack.push(id.to_string());
        for r in crate::import::refs(e.equation.as_deref().unwrap_or("")) {
            match self.find_ref(&r).filter(|t| t.id != id) {
                Some(t) if t.computed => self.visit(&t.id.clone(), c, stack, visited),
                Some(t) => {
                    let key = (t.code.clone(), t.header.clone());
                    if !c.inputs.contains(&key) {
                        c.inputs.push(key);
                    }
                }
                None => c.missing.push(r),
            }
        }
        stack.pop();
        visited.push(id.to_string());
    }

    /// Add an entry; a same-id entry shadows the earlier one (user-over-builtin rule).
    pub fn insert(&mut self, e: Entry) {
        if let Some(&i) = self.by_id.get(&e.id) {
            self.entries[i] = e;
            return;
        }
        let i = self.entries.len();
        self.by_id.insert(e.id.clone(), i);
        self.by_code.entry(e.code.clone()).or_default().push(i);
        self.entries.push(e);
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    pub fn entries_for_code(&self, code: &str) -> Vec<&Entry> {
        self.by_code.get(code).map(|v| v.iter().map(|&i| &self.entries[i]).collect()).unwrap_or_default()
    }

    /// Does any entry of this code carry an enable test?
    pub fn code_has_enable_tests(&self, code: &str) -> bool {
        self.entries_for_code(code).iter().any(|e| e.enable_test.is_some())
    }

    fn evaluator(&mut self, key: &str) -> &mut ExpressionEvaluator {
        self.evaluators.entry(key.to_string()).or_insert_with(|| ExpressionEvaluator::new().expect("evaluator"))
    }

    /// Decode one wire entry from its reply's data bytes (after the `41 xx` echo).
    pub fn decode(&mut self, id: &str, bytes: &[u8]) -> Option<Value> {
        let eq = self.get(id)?.equation.clone()?;
        let key = format!("eq:{id}");
        let vars = HashMap::new();
        match self.evaluator(&key).evaluate_with_bytes_unified(&eq, &vars, bytes) {
            Ok(ExpressionResult::Numeric(v)) if v.is_finite() => Some(Value::Num(v)),
            Ok(ExpressionResult::Numeric(_)) => None,
            Ok(ExpressionResult::Bool(b)) => Some(Value::Num(if b { 1.0 } else { 0.0 })),
            Ok(ExpressionResult::Text(s)) => Some(Value::Text(s)),
            Err(_) => None,
        }
    }

    /// Evaluate an entry's enable test on its reply bytes. `None` = no test.
    pub fn enable_test(&mut self, id: &str, bytes: &[u8]) -> Option<bool> {
        let test = self.get(id)?.enable_test.clone()?;
        let key = format!("en:{id}");
        let vars = HashMap::new();
        Some(match self.evaluator(&key).evaluate_with_bytes_unified(&test, &vars, bytes) {
            Ok(ExpressionResult::Bool(b)) => b,
            Ok(r) => r.as_f64().map(|v| v != 0.0).unwrap_or(false),
            Err(_) => false,
        })
    }

    /// Evaluate a computed entry over the latest values of other entries.
    pub fn compute(&mut self, id: &str, pid_values: &HashMap<String, f64>) -> Option<Value> {
        let eq = self.get(id)?.equation.clone()?;
        let key = format!("eq:{id}");
        let vars = HashMap::new();
        match self.evaluator(&key).evaluate_with_bytes_and_pids_unified(&eq, &vars, &[], pid_values) {
            Ok(ExpressionResult::Numeric(v)) if v.is_finite() => Some(Value::Num(v)),
            Ok(ExpressionResult::Text(s)) => Some(Value::Text(s)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_catalog_has_rpm_with_the_generated_equation() {
        let c = Catalog::from_vault();
        assert!(c.entries.len() > 300, "{}", c.entries.len());
        let rpm = c.get("010C.1").expect("RPM");
        assert_eq!(rpm.unit, "rpm");
        assert_eq!(rpm.max, Some(16383.75));
        assert!(rpm.equation.as_deref().unwrap_or("").contains('A'), "{:?}", rpm.equation);
        assert!(c.get("0100.1").is_none(), "support bitmaps are blacklisted");
        assert!(c.get("0902.1").is_none(), "VIN is blacklisted from the picker");
        // Shipped scope = the publicly listed Mode 01 PIDs (see data/manifest.toml).
        assert!(c.get("01A6.1").is_some(), "odometer is listed");
        assert!(c.get("01B4.1").is_none(), "unlisted PIDs are not shipped");
        assert!(c.entries.iter().any(|e| e.enable_test.is_some()), "enable tests carried");
        assert!(c.entries.iter().all(|e| e.equation.is_some()));
    }

    #[test]
    fn decode_rpm_and_coolant_from_bytes() {
        let mut c = Catalog::from_vault();
        assert_eq!(c.decode("010C.1", &[0x0B, 0xB8]), Some(Value::Num(750.0)));
        assert_eq!(c.decode("0105.1", &[0x7B]), Some(Value::Num(83.0)));
        assert_eq!(c.decode("010D.1", &[0x2F]), Some(Value::Num(47.0)));
    }

    #[test]
    fn enable_tests_hide_missing_sensors() {
        let mut c = Catalog::from_vault();
        // 0178 EGT bank 1: byte A support bits — only sensors 1 and 2 fitted.
        assert_eq!(c.enable_test("0178.1", &[0b0000_0011, 0, 0, 0, 0, 0, 0, 0, 0]), Some(true));
        assert_eq!(c.enable_test("0178.3", &[0b0000_0011, 0, 0, 0, 0, 0, 0, 0, 0]), Some(false));
        assert_eq!(c.enable_test("010C.1", &[0, 0]), None);
    }

    #[test]
    fn computed_entry_evaluates_over_other_values() {
        let mut c = Catalog::from_entries(vec![Entry {
            id: "COMP:Double RPM".into(),
            code: "COMP:Double RPM".into(),
            header: None,
            name: "Double RPM".into(),
            short_name: None,
            unit: "rpm".into(),
            equation: Some("VAL{RPM}*2".into()),
            min: None,
            max: None,
            category: None,
            enable_test: None,
            source: "csv".into(),
            computed: true,
        }]);
        let mut vals = HashMap::new();
        vals.insert("RPM".to_string(), 750.0);
        assert_eq!(c.compute("COMP:Double RPM", &vals), Some(Value::Num(1500.0)));
    }
}
