#![allow(dead_code)]

// src/adapter/handler/connection_management.rs
use super::ConnectionHandler;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::protocol::messages::PusherMessage;
use crate::websocket::SocketId;
use fastwebsockets::{Frame, Payload, WebSocketWrite};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::WriteHalf;
use tracing::warn;

impl ConnectionHandler {
    pub async fn send_message_to_socket(
        &self,
        app_id: &str,
        socket_id: &SocketId,
        message: PusherMessage,
    ) -> Result<()> {
        // Calculate message size for metrics
        let message_size = serde_json::to_string(&message).unwrap_or_default().len();

        // Send the message
        let mut conn_manager = self.connection_manager.lock().await;
        let result = conn_manager.send_message(app_id, socket_id, message).await;

        // Release the lock before metrics
        drop(conn_manager);

        // Track metrics if message was sent successfully
        if result.is_ok()
            && let Some(ref metrics) = self.metrics
        {
            let metrics_locked = metrics.lock().await;
            metrics_locked.mark_ws_message_sent(app_id, message_size);
        }

        result
    }

    /// Broadcast to channel (backward compatible version)
    pub async fn broadcast_to_channel(
        &self,
        app_config: &App,
        channel: &str,
        message: PusherMessage,
        exclude_socket: Option<&SocketId>,
    ) -> Result<()> {
        self.broadcast_to_channel_with_timing(app_config, channel, message, exclude_socket, None)
            .await
    }

    /// Broadcast to channel with optional timing for latency tracking
    pub async fn broadcast_to_channel_with_timing(
        &self,
        app_config: &App,
        channel: &str,
        message: PusherMessage,
        exclude_socket: Option<&SocketId>,
        start_time_ms: Option<f64>,
    ) -> Result<()> {
        // Calculate message size for metrics
        let message_size = serde_json::to_string(&message).unwrap_or_default().len();

        // Get the number of sockets in the channel before sending and send the message
        let (result, target_socket_count) = {
            let mut conn_manager = self.connection_manager.lock().await;

            let socket_count = conn_manager
                .get_channel_socket_count(&app_config.id, channel)
                .await;

            // Adjust for excluded socket
            let target_socket_count = if exclude_socket.is_some() && socket_count > 0 {
                socket_count - 1
            } else {
                socket_count
            };

            let result = conn_manager
                .send(
                    channel,
                    message,
                    exclude_socket,
                    &app_config.id,
                    start_time_ms,
                )
                .await;

            (result, target_socket_count)
        };

        // Track metrics if message was sent successfully
        if result.is_ok()
            && target_socket_count > 0
            && let Some(ref metrics) = self.metrics
        {
            let metrics_locked = metrics.lock().await;
            // Batch metrics update instead of loop for performance
            metrics_locked.mark_ws_messages_sent_batch(
                &app_config.id,
                message_size,
                target_socket_count,
            );

            // Track broadcast latency if we have a start time
            if let Some(start_ms) = start_time_ms {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as f64
                    / 1_000_000.0; // Convert to milliseconds
                let latency_ms = (now_ms - start_ms).max(0.0); // Already in milliseconds with microsecond precision

                metrics_locked.track_broadcast_latency(
                    &app_config.id,
                    channel,
                    target_socket_count,
                    latency_ms,
                );
            }
        }

        result
    }

    pub async fn close_connection(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        code: u16,
        reason: &str,
    ) -> Result<()> {
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, &app_config.id).await {
            let mut conn_locked = conn.0.lock().await;
            conn_locked
                .close(code, reason.to_string())
                .await
                .map_err(|e| Error::Internal(format!("Failed to close connection: {e}")))
        } else {
            warn!("Connection not found for close: {}", socket_id);
            Ok(())
        }
    }

    pub async fn get_channel_member_count(&self, app_config: &App, channel: &str) -> Result<usize> {
        self.connection_manager
            .lock()
            .await
            .get_channel_members(&app_config.id, channel)
            .await
            .map(|members| members.len())
    }

    pub async fn verify_channel_subscription(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        channel: &str,
    ) -> Result<()> {
        let is_subscribed = self
            .connection_manager
            .lock()
            .await
            .is_in_channel(&app_config.id, channel, socket_id)
            .await?;

        if !is_subscribed {
            return Err(Error::ClientEvent(format!(
                "Socket {socket_id} is not subscribed to channel {channel}"
            )));
        }

        Ok(())
    }

    async fn send_error_frame(
        ws_tx: &mut WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>,
        error: &Error,
    ) {
        let error_message = PusherMessage::error(error.close_code(), error.to_string(), None);

        if let Ok(payload) = serde_json::to_string(&error_message) {
            let payload = Payload::from(payload.as_bytes());
            if let Err(e) = ws_tx.write_frame(Frame::text(payload)).await {
                warn!("Failed to send error frame: {e}");
            }
        }

        if let Err(e) = ws_tx
            .write_frame(Frame::close(
                error.close_code(),
                error.to_string().as_bytes(),
            ))
            .await
        {
            warn!("Failed to send close frame: {}", e);
        }
    }
}
