use super::ConnectionHandler;
use crate::error::Result;
use crate::protocol::constants::PONG_TIMEOUT;
use crate::protocol::messages::PusherMessage;
use crate::websocket::SocketId;
use fastwebsockets::Payload;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

impl ConnectionHandler {
    pub async fn setup_initial_timeouts(
        &self,
        socket_id: &SocketId,
        app_config: &crate::app::config::App,
    ) -> Result<()> {
        // Set activity timeout
        self.set_activity_timeout(&app_config.id, socket_id).await?;

        // Set user authentication timeout if required
        if app_config.enable_user_authentication.unwrap_or(false) {
            let auth_timeout = self.server_options.user_authentication_timeout;
            self.set_user_authentication_timeout(&app_config.id, socket_id, auth_timeout)
                .await?;
        }

        Ok(())
    }

    pub async fn set_activity_timeout(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        let socket_id_clone = socket_id.clone();
        let app_id_clone = app_id.to_string();
        let connection_manager = self.connection_manager.clone();
        let activity_timeout = self.server_options.activity_timeout;

        // Clear any existing timeout
        self.clear_activity_timeout(app_id, socket_id).await?;

        let timeout_handle = tokio::spawn(async move {
            // Initial sleep before first check
            sleep(Duration::from_secs(activity_timeout)).await;

            loop {
                // Check if connection still exists and get actual inactivity time
                let mut conn_manager = connection_manager.lock().await;
                let conn = match conn_manager
                    .get_connection(&socket_id_clone, &app_id_clone)
                    .await
                {
                    Some(c) => c,
                    None => {
                        // Connection already cleaned up, nothing to do
                        return;
                    }
                };

                // Check actual time since last activity
                let time_since_activity = {
                    let ws = conn.inner.lock().await;
                    ws.state.time_since_last_ping()
                };

                // If less than activity timeout seconds have passed since last activity, wait more
                if time_since_activity < Duration::from_secs(activity_timeout) {
                    let remaining = Duration::from_secs(activity_timeout) - time_since_activity;
                    debug!(
                        "Socket {} still active ({}s ago), waiting {} more seconds",
                        socket_id_clone,
                        time_since_activity.as_secs(),
                        remaining.as_secs()
                    );
                    drop(conn_manager);
                    sleep(remaining).await;
                    // Continue to check again without additional delay
                    continue;
                }

                // Truly inactive for activity timeout duration, send ping
                let ping_result = {
                    let mut ws = conn.inner.lock().await;
                    // Update connection status to indicate ping sent
                    ws.state.status =
                        crate::websocket::ConnectionStatus::PingSent(std::time::Instant::now());
                    let ping_message = PusherMessage::ping();
                    ws.send_message(&ping_message)
                };

                match ping_result {
                    Ok(_) => {
                        debug!(
                            "Sent ping to socket {} due to activity timeout",
                            socket_id_clone
                        );

                        // Release locks before waiting for pong
                        drop(conn_manager);

                        // Wait for pong response
                        sleep(Duration::from_secs(PONG_TIMEOUT)).await;

                        // Re-acquire lock to check pong status
                        let mut conn_manager = connection_manager.lock().await;
                        if let Some(conn) = conn_manager
                            .get_connection(&socket_id_clone, &app_id_clone)
                            .await
                        {
                            let mut ws = conn.inner.lock().await;
                            // Check if we're still in PingSent state (no pong received)
                            if let crate::websocket::ConnectionStatus::PingSent(ping_time) =
                                ws.state.status
                                && ping_time.elapsed() > Duration::from_secs(PONG_TIMEOUT)
                            {
                                // No pong received, close connection gracefully
                                warn!(
                                    "No pong received from socket {} after ping, closing connection",
                                    socket_id_clone
                                );
                                let _ = ws
                                    .close(4201, "Pong reply not received in time".to_string())
                                    .await;
                            }
                        }
                        // After handling ping/pong, wait full activity timeout before next check
                        drop(conn_manager);
                        sleep(Duration::from_secs(activity_timeout)).await;
                    }
                    Err(e) => {
                        // Connection is broken (e.g., broken pipe)
                        // This is expected when client disconnects abruptly
                        debug!(
                            "Failed to send ping to socket {} (connection likely closed by client): {}",
                            socket_id_clone, e
                        );

                        // Clean up the connection since it's broken
                        // Note: cleanup_connection expects the connection to still exist
                        conn_manager.cleanup_connection(&app_id_clone, conn).await;
                        break; // Exit the loop after cleanup
                    }
                }
            }
        });

        // Store the timeout handle
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, app_id).await {
            let mut ws = conn.inner.lock().await;
            ws.state.timeouts.activity_timeout_handle = Some(timeout_handle);
        }

        Ok(())
    }

    pub async fn clear_activity_timeout(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, app_id).await {
            let mut ws = conn.inner.lock().await;
            ws.state.timeouts.clear_activity_timeout();
        }
        Ok(())
    }

    pub async fn update_activity_timeout(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        // Update last activity time
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, app_id).await {
            let mut ws = conn.inner.lock().await;
            ws.update_activity();
        }
        Ok(())
    }

    pub async fn set_user_authentication_timeout(
        &self,
        app_id: &str,
        socket_id: &SocketId,
        timeout_seconds: u64,
    ) -> Result<()> {
        let socket_id_clone = socket_id.clone();
        let app_id_clone = app_id.to_string();
        let connection_manager = self.connection_manager.clone();

        // Clear any existing auth timeout
        self.clear_user_authentication_timeout(app_id, socket_id)
            .await?;

        let timeout_handle = tokio::spawn(async move {
            sleep(Duration::from_secs(timeout_seconds)).await;

            let mut conn_manager = connection_manager.lock().await;
            if let Some(conn) = conn_manager
                .get_connection(&socket_id_clone, &app_id_clone)
                .await
            {
                let mut ws = conn.inner.lock().await;

                // Check if user is still not authenticated
                if !ws.state.is_authenticated() {
                    let _ = ws
                        .close(
                            4009,
                            "Connection not authorized within timeout.".to_string(),
                        )
                        .await;
                }
            }
        });

        // Store the timeout handle
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, app_id).await {
            let mut ws = conn.inner.lock().await;
            ws.state.timeouts.auth_timeout_handle = Some(timeout_handle);
        }

        Ok(())
    }

    pub async fn clear_user_authentication_timeout(
        &self,
        app_id: &str,
        socket_id: &SocketId,
    ) -> Result<()> {
        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, app_id).await {
            let mut ws = conn.inner.lock().await;
            ws.state.timeouts.clear_auth_timeout();
        }
        Ok(())
    }

    pub async fn handle_ping_frame(
        &self,
        socket_id: &SocketId,
        app_config: &crate::app::config::App,
        payload: Payload<'static>,
    ) -> Result<()> {
        // Update activity and send pong
        self.update_activity_timeout(&app_config.id, socket_id)
            .await?;

        let mut conn_manager = self.connection_manager.lock().await;
        if let Some(conn) = conn_manager.get_connection(socket_id, &app_config.id).await {
            let mut ws = conn.inner.lock().await;
            // Reset connection status to Active when we receive a ping (client is alive)
            ws.state.status = crate::websocket::ConnectionStatus::Active;
            // Send low-level Pong frame in response because the auto response is disabled to allow custom handling
            ws.send_frame(fastwebsockets::Frame::pong(payload))?;
        }

        Ok(())
    }
}
