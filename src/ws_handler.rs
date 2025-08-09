#![allow(unused_variables)]
#![allow(dead_code)]

use crate::adapter::ConnectionHandler;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use fastwebsockets::upgrade;
use serde::Deserialize;
use std::sync::Arc;
use tracing::log::error;

#[derive(Debug, Deserialize)]
pub struct ConnectionQuery {
    protocol: Option<u8>,
    client: Option<String>,
    version: Option<String>,
}

// WebSocket upgrade handler
pub async fn handle_ws_upgrade(
    Path(app_key): Path<String>,
    Query(_params): Query<ConnectionQuery>,
    headers: HeaderMap,
    ws: upgrade::IncomingUpgrade,
    State(handler): State<Arc<ConnectionHandler>>,
) -> impl IntoResponse {
    let (response, fut) = match ws.upgrade() {
        Ok((response, fut)) => (response, fut),
        Err(e) => {
            error!("WebSocket upgrade failed: {e}");
            // Track WebSocket upgrade failure
            if let Some(ref metrics) = handler.metrics {
                let metrics_locked = metrics.lock().await;
                metrics_locked.mark_connection_error(&app_key, "websocket_upgrade_failed");
            }

            return (http::StatusCode::BAD_REQUEST, "WebSocket upgrade failed").into_response();
        }
    };

    // Extract Origin header if present
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    tokio::task::spawn(async move {
        if let Err(e) = handler.handle_socket(fut, app_key.clone(), origin).await {
            error!("Error handling socket: {e}");
            // Only track generic socket handling errors for cases not already tracked
            // Most specific errors (app_not_found, authentication_failed, etc.)
            // are already tracked within handle_socket()
            if let Some(ref metrics) = handler.metrics {
                let metrics_locked = metrics.lock().await;
                match &e {
                    crate::error::Error::ApplicationNotFound
                    | crate::error::Error::ApplicationDisabled
                    | crate::error::Error::Auth(_)
                    | crate::error::Error::InvalidMessageFormat(_)
                    | crate::error::Error::InvalidEventName(_) => {
                        //Do nothing, these errors are already tracked in handle_socket(), don't double-count
                    }
                    // Track other unexpected errors that might not be caught elsewhere
                    _ => {
                        metrics_locked.mark_connection_error(&app_key, "socket_handling_failed");
                    }
                }
            }
        }
    });
    response.into_response()
}
