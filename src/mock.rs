//! omatach's own mock: a responder over the vault's fixtures plus
//! the `extern "C"` send callback `ExternalPlatform` expects. Same wiring as the
//! engine's `mock_send_callback`, owned here — zero engine change. Serves
//! 11-bit (`7Ex`) and 29-bit (`18 DA F1 xx`) fixtures.
//!
//! Gotcha: connector NAMES must not match the OBDLink catalog
//! patterns, or the engine runs the STN periodic ladder against a responder
//! that says "?" to every ST command. "Mock 11-bit" / "Mock 29-bit" don't.
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use obd_session_rs::external_platform::ExternalPlatform;
use obd_session_rs::platform::ConnectorInfo;
use serde::Deserialize;

use crate::vault;

/// One fixture file (data/mock/*.json; shape is omatach's own).
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // name/vin are documentation + future picker detail
pub struct Fixture {
    pub name: String,
    /// "11bit" | "29bit"
    pub addressing: String,
    pub vin: String,
    /// normalized command (no spaces, uppercase) → wire lines (one per frame)
    pub responses: HashMap<String, Vec<String>>,
}

/// Where the last `ATSH` pointed the responder.
#[derive(Debug, Clone)]
enum Target {
    /// `7DF` / `18DB33F1`: every ECU answers.
    Functional,
    /// `ATSH7E0` → only lines from reply header `7E8`.
    Can11 { reply_header: String },
    /// `ATSH18DA10F1` → only lines from `18 DA F1 10`.
    Can29 { ecu: String },
}

pub struct OmatachMockResponder {
    fixtures: HashMap<String, Arc<Fixture>>,
    active: Mutex<Option<Arc<Fixture>>>,
    target: Mutex<Target>,
    pub delay: Duration,
    /// Set by a `04` clear: `03`/`07`/`0A` then answer "no codes" per ECU.
    cleared: AtomicBool,
    /// `OMATACH_MOCK_KOEO=refuse` → `04` answers `7F 04 22` (engine running).
    refuse_clear: bool,
    /// Live drive-cycle simulation (src/sim.rs); `OMATACH_MOCK_STATIC=1` disables.
    simulate: bool,
    /// Show the raw `mock:11bit` / `mock:29bit` rows too (`--dev`, or `--mock`).
    pub dev_mocks: bool,
    /// When the fixture was activated — the simulation clock's zero.
    started: Mutex<Option<std::time::Instant>>,
}

impl OmatachMockResponder {
    /// Load every `mock/*.json` entry from the vault, keyed by its short name (`11bit`).
    pub fn from_vault() -> Self {
        let mut fixtures = HashMap::new();
        for name in vault::names_list() {
            if let Some(key) = name.strip_prefix("mock/").and_then(|n| n.strip_suffix(".json")) {
                let fx: Fixture = vault::json(name);
                fixtures.insert(key.to_string(), Arc::new(fx));
            }
        }
        Self {
            fixtures,
            active: Mutex::new(None),
            target: Mutex::new(Target::Functional),
            delay: Duration::from_millis(20),
            cleared: AtomicBool::new(false),
            refuse_clear: std::env::var("OMATACH_MOCK_KOEO").map(|v| v == "refuse").unwrap_or(false),
            simulate: std::env::var("OMATACH_MOCK_STATIC").map(|v| v != "1").unwrap_or(true),
            dev_mocks: false,
            started: Mutex::new(None),
        }
    }

    /// The key the demo adapter aliases.
    pub const DEMO_KEY: &'static str = "11bit";

    /// The picker entries: always the **Demo adapter** (an alias of the 11-bit
    /// fixture, so anyone without hardware can look around), plus the raw
    /// `mock:<key>` rows in dev mode.
    pub fn connectors(&self) -> Vec<ConnectorInfo> {
        let mut out = vec![ConnectorInfo {
            id: "mock:demo".to_string(),
            name: "Demo adapter (no hardware needed)".to_string(),
            connector_type: "demo".to_string(),
        }];
        if self.dev_mocks {
            let mut keys: Vec<&String> = self.fixtures.keys().collect();
            keys.sort();
            out.extend(keys.into_iter().map(|k| ConnectorInfo {
                id: format!("mock:{k}"),
                name: format!("Mock {} (J1979)", self.fixtures[k].addressing.replace("bit", "-bit")),
                connector_type: "mock".to_string(),
            }));
        }
        out
    }

    pub fn activate(&self, key: &str) -> Result<(), String> {
        let key = if key == "demo" { Self::DEMO_KEY } else { key };
        let fx = self.fixtures.get(key).cloned().ok_or_else(|| format!("no mock fixture named {key:?}"))?;
        *self.active.lock().unwrap() = Some(fx);
        *self.target.lock().unwrap() = Target::Functional;
        self.cleared.store(false, Ordering::SeqCst);
        *self.started.lock().unwrap() = Some(std::time::Instant::now());
        Ok(())
    }

    pub fn deactivate(&self) {
        *self.active.lock().unwrap() = None;
    }

    pub fn is_active(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    /// The reply for one command line (no trailing prompt — the platform adds framing).
    pub fn generate(&self, command: &str) -> String {
        let norm: String = command.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
        let active = self.active.lock().unwrap().clone();

        // Header selection: functional vs physical, 11- or 29-bit.
        if let Some(rest) = norm.strip_prefix("ATSH") {
            let t = if rest == "7DF" || rest == "18DB33F1" {
                Target::Functional
            } else if rest.len() == 3 && rest.starts_with("7E") {
                let n = u16::from_str_radix(rest, 16).unwrap_or(0x7E0) + 8;
                Target::Can11 { reply_header: format!("{n:03X}") }
            } else if rest.len() == 8 && rest.starts_with("18DA") {
                Target::Can29 { ecu: rest[4..6].to_string() }
            } else {
                Target::Functional
            };
            *self.target.lock().unwrap() = t;
            return "OK".to_string();
        }

        match norm.as_str() {
            "ATZ" => return "ELM327 v1.5".to_string(),
            "ATRV" => return "13.2V".to_string(),
            "ATDPN" => {
                return match active.as_ref().map(|f| f.addressing.as_str()) {
                    Some("29bit") => "7".to_string(),
                    _ => "6".to_string(),
                }
            }
            _ => {}
        }
        if norm.starts_with("AT") {
            return "OK".to_string();
        }
        if norm.starts_with("ST") {
            // Not an STN adapter: every ST command is refused → the engine stays on the poll tier.
            return "?".to_string();
        }

        let Some(fx) = active else { return "NO DATA".to_string() };
        // Clear codes: refuse (KOEO not met) or clear and remember it.
        if norm == "04" {
            let headers = ecu_headers(&fx);
            if self.refuse_clear {
                let h = headers.first().cloned().unwrap_or_else(|| "7E8".into());
                return format!("{h} 03 7F 04 22");
            }
            self.cleared.store(true, Ordering::SeqCst);
            return headers.iter().map(|h| format!("{h} 01 44")).collect::<Vec<_>>().join("\r");
        }
        if self.cleared.load(Ordering::SeqCst) && matches!(norm.as_str(), "03" | "07" | "0A") {
            let svc = u8::from_str_radix(&norm, 16).unwrap_or(3) | 0x40;
            return ecu_headers(&fx).iter().map(|h| format!("{h} 02 {svc:02X} 00")).collect::<Vec<_>>().join("\r");
        }
        let Some(lines) = fx.responses.get(&norm) else { return "NO DATA".to_string() };
        // Live values: rewrite the captured mode-01 data bytes from the drive cycle.
        let simulated: Option<Vec<String>> = if self.simulate && norm.len() == 4 && norm.starts_with("01") {
            let t = self.started.lock().unwrap().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
            crate::sim::bytes(&norm[2..], t).map(|data| {
                lines.iter().map(|l| crate::sim::rewrite_line(l, &norm[2..], &data).unwrap_or_else(|| l.clone())).collect()
            })
        } else {
            None
        };
        let lines: &Vec<String> = simulated.as_ref().unwrap_or(lines);
        let target = self.target.lock().unwrap().clone();
        let kept: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|l| match &target {
                Target::Functional => true,
                Target::Can11 { reply_header } => l.starts_with(&format!("{reply_header} ")),
                Target::Can29 { ecu } => l.starts_with(&format!("18 DA F1 {ecu} ")),
            })
            .collect();
        if kept.is_empty() {
            "NO DATA".to_string()
        } else {
            kept.join("\r")
        }
    }
}

/// The reply headers of every ECU in a fixture, from its `0100` lines
/// (`7E8` / `18 DA F1 10`), lowest first.
fn ecu_headers(fx: &Fixture) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in fx.responses.get("0100").into_iter().flatten() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let hdr = if fx.addressing == "29bit" { toks.iter().take(4).cloned().collect::<Vec<_>>().join(" ") } else { toks.first().cloned().unwrap_or("7E8").to_string() };
        if !out.contains(&hdr) {
            out.push(hdr);
        }
    }
    out.sort();
    out
}

/// The context the send callback receives. Leaked for the process lifetime
/// (the platform dereferences the raw pointer from worker threads).
pub struct MockCtx {
    pub responder: Arc<OmatachMockResponder>,
    pub platform: OnceLock<Arc<ExternalPlatform>>,
}

/// `ExternalPlatform`'s send callback for host-routed (`mock:`) connectors:
/// spawn, simulate latency, generate, deliver through `receive_response`.
pub extern "C" fn omatach_mock_send(cmd_ptr: *const c_char, _timeout_ms: u32, context: *mut c_void) {
    if cmd_ptr.is_null() || context.is_null() {
        return;
    }
    let ctx = unsafe { &*(context as *const MockCtx) };
    let command = unsafe { CStr::from_ptr(cmd_ptr) }.to_str().unwrap_or("").to_string();
    let Some(platform) = ctx.platform.get().cloned() else { return };
    let responder = Arc::clone(&ctx.responder);
    let delay = responder.delay;
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let reply = responder.generate(&command);
        platform.receive_response(command, reply);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_adapter_always_listed_and_aliases_the_11bit_fixture() {
        let mut r = OmatachMockResponder::from_vault();
        let ids: Vec<String> = r.connectors().iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids, vec!["mock:demo"]);
        assert_eq!(r.connectors()[0].connector_type, "demo");
        r.dev_mocks = true;
        let ids: Vec<String> = r.connectors().iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids, vec!["mock:demo", "mock:11bit", "mock:29bit"]);
        r.activate("demo").unwrap();
        assert_eq!(r.active.lock().unwrap().as_ref().unwrap().addressing, "11bit");
        assert!(r.activate("nope").is_err());
    }
}
