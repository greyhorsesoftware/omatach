//! `NativePlatform`: a newtype over the engine's
//! `ExternalPlatform` that implements `OBDPlatformInterface` by delegation and
//! overrides only the two places the engine would reach for a Swift callback —
//! discovery's host round (= our mocks) and connect for `mock:` ids. Every real
//! adapter takes the inner's Rust route (`usb:` / `wifi:` / `ble:` / `rfcomm:`),
//! the exact path the mac app runs for USB and WiFi. Zero engine change.
use std::ffi::c_void;
use std::sync::{Arc, OnceLock};

use obd_session_rs::external_platform::ExternalPlatform;
use obd_session_rs::link::dvi::codec::DviFrame;
use obd_session_rs::link::LinkOutcome;
use obd_session_rs::platform::{
    ConnectResult, ConnectionCallback, ConnectionStatus, ConnectorInfo, OBDDataCallback, OBDPlatformInterface,
    ReplyDecoder,
};
use obd_session_rs::session_logger::SessionLogger;

use crate::mock::{omatach_mock_send, MockCtx, OmatachMockResponder};

pub struct NativePlatform {
    inner: Arc<ExternalPlatform>,
    mock: Arc<OmatachMockResponder>,
}

impl NativePlatform {
    /// Build the inner platform with omatach's mock send callback, attach it, wrap it.
    pub fn new(dev_mocks: bool) -> Arc<Self> {
        let mut responder = OmatachMockResponder::from_vault();
        responder.dev_mocks = dev_mocks;
        let responder = Arc::new(responder);
        let ctx: &'static MockCtx = Box::leak(Box::new(MockCtx { responder: Arc::clone(&responder), platform: OnceLock::new() }));
        let inner = Arc::new(ExternalPlatform::new(omatach_mock_send, ctx as *const MockCtx as *mut c_void));
        let _ = ctx.platform.set(Arc::clone(&inner));
        inner.attach_self();
        Arc::new(Self { inner, mock: responder })
    }

    #[allow(dead_code)]
    pub fn mock(&self) -> &OmatachMockResponder {
        &self.mock
    }
}

impl OBDPlatformInterface for NativePlatform {
    // ---- the two overrides ------------------------------------------------

    /// Host round = the mocks. The discovery engine merges the Rust probe on top.
    fn scan_connectors_round(&self, callback: Box<dyn FnOnce(Vec<ConnectorInfo>) + Send>) {
        callback(self.mock.connectors());
    }

    fn connect_to(&self, connector_id: &str, callback: Box<dyn FnOnce(ConnectResult) + Send>) {
        if let Some(key) = connector_id.strip_prefix("mock:") {
            match self.mock.activate(key) {
                Ok(()) => {
                    self.inner.update_connection_status(ConnectionStatus::Connected, None);
                    callback(ConnectResult::Connected);
                }
                Err(reason) => callback(ConnectResult::Failed { reason }),
            }
        } else {
            self.inner.connect_to(connector_id, callback)
        }
    }

    fn abandon_pending_connect(&self) {
        self.inner.abandon_pending_connect()
    }

    fn disconnect_from(&self) {
        if self.mock.is_active() {
            self.mock.deactivate();
            self.inner.update_connection_status(ConnectionStatus::Disconnected, Some("User disconnected".to_string()));
        } else {
            self.inner.disconnect_from()
        }
    }

    // ---- pure delegation ---------------------------------------------------

    fn set_wire_log(&self, logger: Arc<SessionLogger>) {
        self.inner.set_wire_log(logger)
    }
    fn send_command(&self, command: &str, timeout_ms: u32) {
        self.inner.send_command(command, timeout_ms)
    }
    fn set_completion_callback(&self, callback: Option<Box<dyn Fn(String) + Send + Sync>>) {
        self.inner.set_completion_callback(callback)
    }
    fn set_data_callback(&self, callback: Option<OBDDataCallback>) {
        self.inner.set_data_callback(callback)
    }
    fn set_connection_callback(&self, callback: Option<ConnectionCallback>) {
        self.inner.set_connection_callback(callback)
    }
    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
    fn connection_status(&self) -> ConnectionStatus {
        self.inner.connection_status()
    }
    fn note_connection_status(&self, status: ConnectionStatus) {
        self.inner.note_connection_status(status)
    }
    fn enter_monitor_mode(&self, filter: Option<&str>, on_line: Box<dyn Fn(String) + Send + Sync>) -> Result<(), String> {
        self.inner.enter_monitor_mode(filter, on_line)
    }
    fn exit_monitor_mode(&self) {
        self.inner.exit_monitor_mode()
    }
    fn write_raw(&self, line: &str) {
        self.inner.write_raw(line)
    }
    fn set_byte_mode(&self, on: bool) {
        self.inner.set_byte_mode(on)
    }
    fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        self.inner.write_bytes(bytes)
    }
    fn set_frame_sink(&self, sink: Option<Arc<dyn Fn(DviFrame) + Send + Sync>>) {
        self.inner.set_frame_sink(sink)
    }
    fn set_command_encoder(&self, enc: Option<Arc<dyn Fn(&str) -> Result<Option<Vec<u8>>, String> + Send + Sync>>) {
        self.inner.set_command_encoder(enc)
    }
    fn complete_command(&self, command: &str, outcome: LinkOutcome) {
        self.inner.complete_command(command, outcome)
    }
    fn set_reply_decoder(&self, dec: Option<ReplyDecoder>) {
        self.inner.set_reply_decoder(dec)
    }
    fn set_frame_timestamps(&self, on: bool) {
        self.inner.set_frame_timestamps(on)
    }
}
