#![allow(async_fn_in_trait)]

// src/websocket.rs
use crate::app::config::App;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};
use crate::protocol::messages::PusherMessage;
use fastwebsockets::{Frame, Payload, WebSocketWrite};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::WriteHalf;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct SocketId(pub String);

impl std::fmt::Display for SocketId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for SocketId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Default for SocketId {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq<String> for SocketId {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl SocketId {
    pub fn new() -> Self {
        Self(Self::generate_socket_id())
    }

    fn generate_socket_id() -> String {
        let mut rng = rand::rng();
        let min: u64 = 0;
        let max: u64 = 10_000_000_000;

        let mut random_number = || rng.random_range(min..=max);
        format!("{}.{}", random_number(), random_number())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserInfo {
    pub id: String,
    pub watchlist: Option<Vec<String>>,
    pub info: Option<Value>,
}

#[derive(Debug)]
pub struct ConnectionTimeouts {
    pub activity_timeout_handle: Option<JoinHandle<()>>,
    pub auth_timeout_handle: Option<JoinHandle<()>>,
}

impl Default for ConnectionTimeouts {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionTimeouts {
    pub fn new() -> Self {
        Self {
            activity_timeout_handle: None,
            auth_timeout_handle: None,
        }
    }

    pub fn clear_activity_timeout(&mut self) {
        if let Some(handle) = self.activity_timeout_handle.take() {
            handle.abort();
        }
    }

    pub fn clear_auth_timeout(&mut self) {
        if let Some(handle) = self.auth_timeout_handle.take() {
            handle.abort();
        }
    }

    pub fn clear_all(&mut self) {
        self.clear_activity_timeout();
        self.clear_auth_timeout();
    }
}

impl Drop for ConnectionTimeouts {
    fn drop(&mut self) {
        self.clear_all();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Active,
    PingSent(Instant),
    Closing,
    Closed,
}

#[derive(Debug)]
pub struct ConnectionState {
    pub socket_id: SocketId,
    pub app: Option<App>,
    pub subscribed_channels: HashSet<String>,
    pub user_id: Option<String>,
    pub user_info: Option<UserInfo>,
    pub last_ping: Instant,
    pub presence: Option<HashMap<String, PresenceMemberInfo>>,
    pub user: Option<Value>,
    pub timeouts: ConnectionTimeouts,
    pub status: ConnectionStatus,
    pub disconnecting: bool,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            socket_id: SocketId::new(),
            app: None,
            subscribed_channels: HashSet::new(),
            user_id: None,
            user_info: None,
            last_ping: Instant::now(),
            presence: None,
            user: None,
            timeouts: ConnectionTimeouts::new(),
            status: ConnectionStatus::Active,
            disconnecting: false,
        }
    }

    pub fn with_socket_id(socket_id: SocketId) -> Self {
        Self {
            socket_id,
            app: None,
            subscribed_channels: HashSet::new(),
            user_id: None,
            user_info: None,
            last_ping: Instant::now(),
            presence: None,
            user: None,
            timeouts: ConnectionTimeouts::new(),
            status: ConnectionStatus::Active,
            disconnecting: false,
        }
    }

    pub fn is_presence(&self) -> bool {
        self.presence.is_some()
    }

    pub fn is_subscribed(&self, channel: &str) -> bool {
        self.subscribed_channels.contains(channel)
    }

    pub fn add_subscription(&mut self, channel: String) {
        self.subscribed_channels.insert(channel);
    }

    pub fn remove_subscription(&mut self, channel: &str) -> bool {
        self.subscribed_channels.remove(channel)
    }

    pub fn update_ping(&mut self) {
        self.last_ping = Instant::now();
    }

    pub fn get_app_key(&self) -> String {
        self.app
            .as_ref()
            .map(|app| app.key.clone())
            .unwrap_or_default()
    }

    pub fn time_since_last_ping(&self) -> std::time::Duration {
        self.last_ping.elapsed()
    }

    pub fn is_authenticated(&self) -> bool {
        self.user.is_some()
    }

    pub fn clear_timeouts(&mut self) {
        self.timeouts.clear_all();
    }
}

impl PartialEq for ConnectionState {
    fn eq(&self, other: &Self) -> bool {
        self.socket_id == other.socket_id
    }
}

// Message sender for async message handling
#[derive(Debug)]
pub struct MessageSender {
    sender: mpsc::UnboundedSender<Frame<'static>>,
    _receiver_handle: JoinHandle<()>,
}

#[derive(Debug, Clone, Copy)]
enum SocketOperation {
    WriteFrame,
    SendCloseFrame,
}

impl std::fmt::Display for SocketOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketOperation::WriteFrame => write!(f, "write frame to WebSocket"),
            SocketOperation::SendCloseFrame => write!(f, "send close frame"),
        }
    }
}

impl SocketOperation {
    fn is_close_operation(&self) -> bool {
        matches!(self, SocketOperation::SendCloseFrame)
    }
}

impl MessageSender {
    fn is_connection_error(error: &fastwebsockets::WebSocketError) -> bool {
        // For now, let's use a different approach to check if it's an IO error
        // We'll pattern match on the error's source or check if it contains IO error
        if let Some(source) = StdError::source(error) {
            if let Some(io_err) = source.downcast_ref::<std::io::Error>() {
                matches!(
                    io_err.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                )
            } else {
                false
            }
        } else {
            false
        }
    }

    fn log_connection_error(
        error: &fastwebsockets::WebSocketError,
        operation: SocketOperation,
        frame_count: usize,
        is_shutting_down: bool,
    ) {
        let is_conn_err = Self::is_connection_error(error);

        if is_conn_err && is_shutting_down {
            debug!("{} failed during shutdown (expected): {}", operation, error);
        } else if is_conn_err && frame_count <= 2 {
            warn!(
                "Early connection {} failed (after {} frames): {}",
                operation, frame_count, error
            );
        } else if is_conn_err {
            warn!(
                "Connection {} failed during operation (after {} frames): {}",
                operation, frame_count, error
            );
        } else if operation.is_close_operation() {
            warn!("Failed to {}: {}", operation, error);
        } else {
            error!("Failed to {}: {}", operation, error);
        }
    }

    pub fn new(mut socket: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Frame<'static>>();

        let receiver_handle = tokio::spawn(async move {
            let mut frame_count = 0;
            let mut is_shutting_down = false;

            while let Some(frame) = receiver.recv().await {
                frame_count += 1;

                // Detect if this is a close frame (indicates shutdown)
                if matches!(frame.opcode, fastwebsockets::OpCode::Close) {
                    is_shutting_down = true;
                }

                if let Err(e) = socket.write_frame(frame).await {
                    Self::log_connection_error(
                        &e,
                        SocketOperation::WriteFrame,
                        frame_count,
                        is_shutting_down,
                    );
                    break;
                }
            }

            // Try to close gracefully
            if let Err(e) = socket
                .write_frame(Frame::close(1000, b"Normal closure"))
                .await
            {
                Self::log_connection_error(&e, SocketOperation::SendCloseFrame, frame_count, true);
            }
        });

        Self {
            sender,
            _receiver_handle: receiver_handle,
        }
    }

    pub fn send(&self, frame: Frame<'static>) -> Result<()> {
        self.sender
            .send(frame)
            .map_err(|_| Error::Connection("Message channel closed".into()))
    }

    pub fn send_json<T: serde::Serialize>(&self, message: &T) -> Result<()> {
        let payload = serde_json::to_vec(message)
            .map_err(|e| Error::InvalidMessageFormat(format!("Serialization failed: {e}")))?;

        let frame = Frame::text(Payload::from(payload));
        self.send(frame)
    }

    pub fn send_raw_bytes(&self, bytes: Arc<Vec<u8>>) -> Result<()> {
        let frame = Frame::text(Payload::from(bytes.as_ref().clone()));
        self.send(frame)
    }

    pub fn send_text(&self, text: String) -> Result<()> {
        let frame = Frame::text(Payload::from(text.into_bytes()));
        self.send(frame)
    }

    pub fn send_close(&self, code: u16, reason: &str) -> Result<()> {
        let frame = Frame::close(code, reason.as_bytes());
        self.send(frame)
    }
}

pub struct WebSocket {
    pub state: ConnectionState,
    pub message_sender: MessageSender,
}

impl WebSocket {
    pub fn new(socket_id: SocketId, socket: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>) -> Self {
        Self {
            state: ConnectionState::with_socket_id(socket_id),
            message_sender: MessageSender::new(socket),
        }
    }

    pub fn get_socket_id(&self) -> &SocketId {
        &self.state.socket_id
    }

    pub async fn close(&mut self, code: u16, reason: String) -> Result<()> {
        // Check if already closing or closed
        match self.state.status {
            ConnectionStatus::Closing | ConnectionStatus::Closed => {
                debug!("Connection already closing or closed, skipping close frames");
                return Ok(());
            }
            _ => {}
        }

        // Update connection status
        self.state.status = ConnectionStatus::Closing;

        // Send error message first if it's an error closure
        if code >= 4000 {
            let error_message = PusherMessage::error(code, reason.clone(), None);
            if let Err(e) = self.message_sender.send_json(&error_message) {
                warn!("Failed to send error message before close: {}", e);
            }
        }

        // Send close frame
        self.message_sender.send_close(code, &reason)?;

        // Clear any active timeouts
        self.state.clear_timeouts();

        // Mark as closed
        self.state.status = ConnectionStatus::Closed;

        Ok(())
    }

    pub fn send_message(&self, message: &PusherMessage) -> Result<()> {
        // Check if connection is in a valid state to send messages
        match self.state.status {
            ConnectionStatus::Active | ConnectionStatus::PingSent(_) => {
                self.message_sender.send_json(message)
            }
            ConnectionStatus::Closing | ConnectionStatus::Closed => Err(Error::ConnectionClosed(
                "Cannot send message on closed connection".to_string(),
            )),
        }
    }

    pub fn send_raw_bytes(&self, bytes: Arc<Vec<u8>>) -> Result<()> {
        // Check if connection is in a valid state to send messages
        match self.state.status {
            ConnectionStatus::Active | ConnectionStatus::PingSent(_) => {
                self.message_sender.send_raw_bytes(bytes)
            }
            ConnectionStatus::Closing | ConnectionStatus::Closed => Err(Error::ConnectionClosed(
                "Cannot send message on closed connection".to_string(),
            )),
        }
    }

    pub fn send_text(&self, text: String) -> Result<()> {
        self.message_sender.send_text(text)
    }

    pub fn send_frame(&self, frame: Frame<'static>) -> Result<()> {
        self.message_sender.send(frame)
    }

    pub fn is_connected(&self) -> bool {
        matches!(
            self.state.status,
            ConnectionStatus::Active | ConnectionStatus::PingSent(_)
        )
    }

    pub fn update_activity(&mut self) {
        self.state.update_ping();
    }

    pub fn set_user_info(&mut self, user_info: UserInfo) {
        self.state.user_id = Some(user_info.id.clone());
        self.state.user_info = Some(user_info.clone());

        // Also set the user Value for backward compatibility
        if let Some(info) = &user_info.info {
            self.state.user = Some(info.clone());
        }
    }

    pub fn add_presence_info(&mut self, channel: String, member_info: PresenceMemberInfo) {
        if self.state.presence.is_none() {
            self.state.presence = Some(HashMap::new());
        }

        if let Some(ref mut presence) = self.state.presence {
            presence.insert(channel, member_info);
        }
    }

    pub fn remove_presence_info(&mut self, channel: &str) -> Option<PresenceMemberInfo> {
        self.state.presence.as_mut()?.remove(channel)
    }

    pub fn subscribe_to_channel(&mut self, channel: String) {
        self.state.add_subscription(channel);
    }

    pub fn unsubscribe_from_channel(&mut self, channel: &str) -> bool {
        self.state.remove_subscription(channel)
    }

    pub fn is_subscribed_to(&self, channel: &str) -> bool {
        self.state.is_subscribed(channel)
    }

    pub fn get_subscribed_channels(&self) -> &HashSet<String> {
        &self.state.subscribed_channels
    }
}

impl PartialEq for WebSocket {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl Hash for WebSocket {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.state.socket_id.hash(state);
    }
}

#[derive(Clone)]
pub struct WebSocketRef(pub Arc<Mutex<WebSocket>>);

impl WebSocketRef {
    pub fn new(websocket: WebSocket) -> Self {
        Self(Arc::new(Mutex::new(websocket)))
    }

    pub async fn send_message(&self, message: &PusherMessage) -> Result<()> {
        let ws = self.0.lock().await;
        ws.send_message(message)
    }

    pub async fn send_raw_message(&self, bytes: Arc<Vec<u8>>) -> Result<()> {
        let ws = self.0.lock().await;
        ws.send_raw_bytes(bytes)
    }

    pub async fn close(&self, code: u16, reason: String) -> Result<()> {
        let mut ws = self.0.lock().await;
        ws.close(code, reason).await
    }

    pub async fn get_socket_id(&self) -> SocketId {
        let ws = self.0.lock().await;
        ws.get_socket_id().clone()
    }

    pub async fn is_subscribed_to(&self, channel: &str) -> bool {
        let ws = self.0.lock().await;
        ws.is_subscribed_to(channel)
    }

    pub async fn get_user_id(&self) -> Option<String> {
        let ws = self.0.lock().await;
        ws.state.user_id.clone()
    }

    pub async fn update_activity(&self) {
        let mut ws = self.0.lock().await;
        ws.update_activity();
    }

    pub async fn subscribe_to_channel(&self, channel: String) {
        let mut ws = self.0.lock().await;
        ws.subscribe_to_channel(channel);
    }

    pub async fn unsubscribe_from_channel(&self, channel: &str) -> bool {
        let mut ws = self.0.lock().await;
        ws.unsubscribe_from_channel(channel)
    }
}

impl Hash for WebSocketRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Use the raw pointer address of the Arc for hashing
        let ptr = Arc::as_ptr(&self.0) as *const () as usize;
        ptr.hash(state);
    }
}

impl PartialEq for WebSocketRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WebSocketRef {}

impl Debug for WebSocketRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketRef")
            .field("ptr", &Arc::as_ptr(&self.0))
            .finish()
    }
}

// Helper trait for easier WebSocket operations
pub trait WebSocketExt {
    async fn send_pusher_message(&self, message: PusherMessage) -> Result<()>;
    async fn send_error(&self, code: u16, message: String, channel: Option<String>) -> Result<()>;
    async fn send_pong(&self) -> Result<()>;
}

impl WebSocketExt for WebSocketRef {
    async fn send_pusher_message(&self, message: PusherMessage) -> Result<()> {
        self.send_message(&message).await
    }

    async fn send_error(&self, code: u16, message: String, channel: Option<String>) -> Result<()> {
        let error_msg = PusherMessage::error(code, message, channel);
        self.send_message(&error_msg).await
    }

    async fn send_pong(&self) -> Result<()> {
        let pong_msg = PusherMessage::pong();
        self.send_message(&pong_msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_id_generation() {
        let id1 = SocketId::new();
        let id2 = SocketId::new();

        assert_ne!(id1, id2);
        assert!(id1.0.contains('.'));
        assert!(id2.0.contains('.'));
    }

    #[test]
    fn test_connection_state() {
        let mut state = ConnectionState::new();

        // Test subscription management
        assert!(!state.is_subscribed("test-channel"));
        state.add_subscription("test-channel".to_string());
        assert!(state.is_subscribed("test-channel"));
        assert!(state.remove_subscription("test-channel"));
        assert!(!state.is_subscribed("test-channel"));
    }

    #[test]
    fn test_socket_id_display() {
        let id = SocketId("123.456".to_string());
        assert_eq!(format!("{id}"), "123.456");
        assert_eq!(id.as_ref(), "123.456");
    }
}
