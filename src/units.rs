//! Units (decided 2026-09-11): the vault carries the mac app's
//! `unitconversions.json` (metric → imperial linear map); a `units` preference
//! in `~/.config/omatach/config.toml` plus the `u` key applies it at render
//! time. Catalog values stay metric; min/max convert with the value.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vault;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

#[derive(Debug, Clone, Deserialize)]
struct Conv {
    target: String,
    m: f64,
    b: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ConvFile {
    #[serde(rename = "metricToImperial")]
    metric_to_imperial: HashMap<String, Conv>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub units: UnitSystem,
    #[serde(default)]
    pub ui: UiConfig,
}

/// `[ui]` — `braille = false` for terminals that render braille badly
/// (dials fall back to bars); `OMATACH_NO_BRAILLE=1` does the same.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub braille: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { braille: true }
    }
}

/// Read the config file once (units + ui).
pub fn load_config(config_dir: Option<&Path>) -> Config {
    config_dir
        .map(|d| d.join("config.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .unwrap_or_default()
}

pub struct Units {
    pub system: UnitSystem,
    table: HashMap<String, Conv>,
    config_path: Option<PathBuf>,
    ui: UiConfig,
}

impl Units {
    pub fn load(config_dir: Option<&Path>) -> Self {
        let file: ConvFile = vault::json(vault::names::UNITCONVERSIONS_JSON);
        let config_path = config_dir.map(|d| d.join("config.toml"));
        let cfg = load_config(config_dir);
        Self { system: cfg.units, table: file.metric_to_imperial, config_path, ui: cfg.ui }
    }

    pub fn toggle(&mut self) {
        self.system = match self.system {
            UnitSystem::Metric => UnitSystem::Imperial,
            UnitSystem::Imperial => UnitSystem::Metric,
        };
        if let Some(p) = &self.config_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, toml::to_string(&Config { units: self.system, ui: self.ui.clone() }).unwrap_or_default());
        }
    }

    /// May the dashboard draw braille (dials)? Config `[ui] braille`, or `OMATACH_NO_BRAILLE=1`.
    pub fn braille(&self) -> bool {
        self.ui.braille
    }

    /// Override the braille setting (the `OMATACH_NO_BRAILLE=1` env is applied at startup in main).
    pub fn set_braille(&mut self, on: bool) {
        self.ui.braille = on;
    }

    /// Convert a value in `unit` for display: `(value, unit)` in the active system.
    pub fn display(&self, value: f64, unit: &str) -> (f64, String) {
        if self.system == UnitSystem::Metric {
            return (value, unit.to_string());
        }
        match self.table.get(&unit.trim().to_lowercase()) {
            Some(c) => (value * c.m + c.b, c.target.clone()),
            None => (value, unit.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imperial_maps_the_common_units_and_leaves_others() {
        let mut u = Units::load(None);
        assert_eq!(u.system, UnitSystem::Metric);
        assert_eq!(u.display(91.0, "°C"), (91.0, "°C".into()));
        u.toggle();
        let (f, unit) = u.display(100.0, "°C");
        assert!((f - 212.0).abs() < 0.01, "{f}");
        assert_eq!(unit, "°F");
        let (mph, unit) = u.display(100.0, "km/h");
        assert!((mph - 62.137).abs() < 0.01);
        assert_eq!(unit, "mph");
        assert_eq!(u.display(3000.0, "rpm"), (3000.0, "rpm".into()));
    }
}
