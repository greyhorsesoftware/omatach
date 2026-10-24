//! The engine seam: builds the platform + session, and
//! turns the engine's JSON callback stream into typed [`EngineEvent`]s on an
//! mpsc channel the UI thread drains. Every call here is an existing
//! `SessionAPIHandle` method; nothing reaches into engine internals.
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use obd_session_rs::platform::{ConnectorInfo, OBDPlatformInterface};
use obd_session_rs::session_manager::SessionAPIHandle;
use obd_session_rs::vehicle_info::VehicleInfo;
use obd_session_rs::{OBDSessionConfig, OBDSessionManager};
use serde_json::Value;

use crate::platform::NativePlatform;
use crate::vault;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoveryFacts {
    pub protocol: Option<String>,
    pub vin: Option<String>,
    pub pid_count: Option<u64>,
    pub ecu_count: Option<u64>,
}

/// (Payload fields not every screen reads yet are allowed dead.)
#[derive(Debug)]
#[allow(dead_code)]
pub enum EngineEvent {
    /// A `connection_progress` step (message key + optional detail), humanized by the UI.
    Progress { message: String, detail: Option<String> },
    /// The picker list (waiting_for_selection / connector_list).
    Connectors(Vec<ConnectorInfo>),
    /// Incremental identify facts (C7 live scan panel).
    Discovery(DiscoveryFacts),
    VehicleInfo(Box<VehicleInfo>),
    Connected,
    Disconnected(Option<String>),
    ConnectionFailed(Option<String>),
    /// A command reply.
    ObdData { command: String, data: String, error: Option<String>, parsed: Option<Value> },
}

pub struct Engine {
    platform: Arc<NativePlatform>,
    session: Option<OBDSessionManager>,
    api: SessionAPIHandle,
    pub rx: Receiver<EngineEvent>,
    /// Dashboard subscriptions, one per target header (`None` = functional), per connection.
    subs: Mutex<std::collections::HashMap<Option<String>, Uuid>>,
}

impl Engine {
    /// Build the platform and session; hand the vault's adapter catalog to the engine.
    pub fn start(cache_dir: Option<std::path::PathBuf>, dev_mocks: bool) -> Result<Self, String> {
        let platform = NativePlatform::new(dev_mocks);
        let (tx, rx) = mpsc::channel::<EngineEvent>();
        let mut config = OBDSessionConfig::default();
        config.cache_path = cache_dir.map(|p| p.to_string_lossy().into_owned());
        let session = OBDSessionManager::new(platform.clone() as Arc<dyn OBDPlatformInterface>, config, move |json| {
            if let Ok(v) = serde_json::from_str::<Value>(&json) {
                for ev in parse(v) {
                    let _ = tx.send(ev);
                }
            }
        })
        .map_err(|e| format!("session: {e:?}"))?;
        let api = session.api_handle();
        let catalog = std::str::from_utf8(vault::bytes(vault::names::ADAPTERS_JSON)).map_err(|e| e.to_string())?;
        api.set_adapter_catalog(catalog)?;
        Ok(Self { platform, session: Some(session), api, rx, subs: Mutex::new(std::collections::HashMap::new()) })
    }

    /// Start (or restart) discovery — the engine owns the scan loop until a selection lands.
    pub fn scan(&self) {
        self.subs.lock().unwrap().clear();
        self.api.connect_to_controller();
    }

    pub fn select(&self, connector_id: &str) {
        self.api.select_connector(self.platform.clone() as Arc<dyn OBDPlatformInterface>, connector_id);
    }

    pub fn cancel(&self) {
        self.api.cancel_connect_attempt();
    }

    pub fn disconnect(&self) {
        self.subs.lock().unwrap().clear();
        self.api.disconnect_and_drain(self.platform.clone() as Arc<dyn OBDPlatformInterface>);
    }

    /// Poll these `(code, header)` targets continuously — one subscription per
    /// header (the engine issues `ATSH<header> <pid>` for a targeted subscription
    /// and dedupes members across them).
    pub fn subscribe(&self, targets: Vec<(String, Option<String>)>) {
        let mut by_header: std::collections::HashMap<Option<String>, Vec<String>> = std::collections::HashMap::new();
        for (code, header) in targets {
            by_header.entry(header).or_default().push(code);
        }
        let mut subs = self.subs.lock().unwrap();
        for (header, codes) in by_header {
            match subs.get(&header).copied() {
                Some(id) => {
                    if let Err(e) = self.api.add_pids_to_subscription(id, codes, header.clone()) {
                        eprintln!("subscribe: add failed: {e:?}");
                    }
                    // A (re)start is the engine's cue to climb back to its fastest
                    // tier (STN periodic). Without it a dashboard that was emptied
                    // — leaving the screen — comes back on plain polling for good.
                    if let Err(e) = self.api.start_subscription(id) {
                        eprintln!("subscribe: restart failed: {e:?}");
                    }
                }
                None => {
                    let name = format!("dashboard{}", header.as_deref().map(|h| format!("@{h}")).unwrap_or_default());
                    match self.api.create_subscription(Some(name), codes, header.clone()) {
                        Ok(id) => {
                            if let Err(e) = self.api.start_subscription(id) {
                                eprintln!("subscribe: start failed: {e:?}");
                            }
                            subs.insert(header, id);
                        }
                        Err(e) => eprintln!("subscribe: create failed: {e:?}"),
                    }
                }
            }
        }
    }

    pub fn unsubscribe(&self, targets: Vec<(String, Option<String>)>) {
        let subs = self.subs.lock().unwrap();
        for (code, header) in targets {
            if let Some(id) = subs.get(&header) {
                if let Err(e) = self.api.remove_pids_from_subscription(*id, vec![code]) {
                    eprintln!("unsubscribe failed: {e:?}");
                }
            }
        }
    }

    /// One-shot reads (enable-test probes); replies arrive as `obd_data`.
    pub fn probe(&self, codes: Vec<String>) {
        for c in codes {
            if let Err(e) = self.api.send_command(&c, 3000) {
                eprintln!("probe {c}: {e:?}");
            }
        }
    }

    #[allow(dead_code)]
    pub fn send_command(&self, command: &str, timeout_ms: u32) -> Result<(), String> {
        self.api.send_command(command, timeout_ms).map_err(|e| format!("{e:?}"))
    }

    pub fn shutdown(mut self) {
        if let Some(s) = self.session.take() {
            let _ = s.shutdown();
        }
    }
}

/// Map one JSON message to zero or more events.
fn parse(v: Value) -> Vec<EngineEvent> {
    let ty = v["type"].as_str().unwrap_or("");
    let p = &v["payload"];
    match ty {
        "connection_progress" => {
            let message = p["message"].as_str().unwrap_or("").to_string();
            let detail = p["detail"].as_str().map(str::to_string);
            let mut out = Vec::new();
            match message.as_str() {
                "waiting_for_selection" | "connector_list" => {
                    let list: Vec<ConnectorInfo> = serde_json::from_value(p["connectors"].clone()).unwrap_or_default();
                    out.push(EngineEvent::Connectors(list));
                }
                "discovery" => {
                    let d = &p["discovery"];
                    out.push(EngineEvent::Discovery(DiscoveryFacts {
                        protocol: d["protocol"].as_str().map(str::to_string),
                        vin: d["vin"].as_str().map(str::to_string),
                        pid_count: d["pid_count"].as_u64(),
                        ecu_count: d["ecu_count"].as_u64(),
                    }));
                }
                "vehicle_info" => {
                    if let Ok(vi) = serde_json::from_value::<VehicleInfo>(p["vehicle_info"].clone()) {
                        out.push(EngineEvent::VehicleInfo(Box::new(vi)));
                    }
                }
                "connected" => out.push(EngineEvent::Connected),
                "disconnected" => out.push(EngineEvent::Disconnected(detail.clone())),
                "connection_failed" => out.push(EngineEvent::ConnectionFailed(detail.clone())),
                _ => out.push(EngineEvent::Progress { message, detail }),
            }
            out
        }
        "obd_data" => vec![EngineEvent::ObdData {
            command: p["command"].as_str().unwrap_or("").to_string(),
            data: p["data"].as_str().unwrap_or("").to_string(),
            error: p["error"].as_str().map(str::to_string),
            parsed: if p["parsed"].is_null() { None } else { Some(p["parsed"].clone()) },
        }],
        _ => Vec::new(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Count `obd_data` replies for `code` over `window`.
    fn replies(e: &Engine, code: &str, window: Duration) -> usize {
        let end = Instant::now() + window;
        let mut n = 0;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok(EngineEvent::ObdData { command, .. }) = e.rx.recv_timeout(left) {
                n += (command == code) as usize;
            }
        }
        n
    }

    /// Against the real engine + demo adapter: unsubscribing every PID makes
    /// the wire go quiet (leaving the dashboard), and subscribing again
    /// resumes polling (coming back). Ignored by default: the connect runs a
    /// real discovery round (Bluetooth scan included) and takes ~13 s —
    /// `cargo test engine::tests -- --ignored`.
    #[test]
    #[ignore]
    fn unsubscribe_silences_polling_and_resubscribe_resumes() {
        let e = Engine::start(None, false).unwrap();
        e.scan();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut selected = false;
        loop {
            let left = deadline.checked_duration_since(Instant::now()).expect("demo adapter connects");
            match e.rx.recv_timeout(left) {
                Ok(EngineEvent::Connectors(_)) if !selected => {
                    selected = true;
                    e.select("mock:demo");
                }
                Ok(EngineEvent::Connected) => break,
                Ok(EngineEvent::ConnectionFailed(r)) => panic!("connect failed: {r:?}"),
                _ => {}
            }
        }
        let rpm = vec![("010C".to_string(), None)];
        e.subscribe(rpm.clone());
        assert!(replies(&e, "010C", Duration::from_secs(2)) > 0, "polling while subscribed");

        e.unsubscribe(rpm.clone());
        let _ = replies(&e, "010C", Duration::from_millis(700)); // let the in-flight poll land
        assert_eq!(replies(&e, "010C", Duration::from_secs(2)), 0, "quiet after unsubscribe");

        e.subscribe(rpm);
        assert!(replies(&e, "010C", Duration::from_secs(2)) > 0, "polling resumes");
        e.disconnect();
        e.shutdown();
    }
}
