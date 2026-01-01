#![allow(unused_variables)]
#![allow(dead_code)]

use crate::adapter::ConnectionHandler;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use fastwebsockets::upgrade;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, warn};

#[derive(Debug, Deserialize)]
pub struct ConnectionQuery {
    protocol: Option<u8>,
    client: Option<String>,
    version: Option<String>,
}

/// Extract client IP from headers or connection info.
/// Respects X-Forwarded-For and X-Real-IP headers for proxy setups.
fn extract_client_ip(headers: &HeaderMap, connect_info: Option<SocketAddr>) -> Option<String> {
    // Try X-Forwarded-For first (may contain multiple IPs, take the first one)
    if let Some(xff) = headers.get("x-forwarded-for") {
        if let Ok(xff_str) = xff.to_str() {
            if let Some(first_ip) = xff_str.split(',').next() {
                let ip = first_ip.trim();
                if !ip.is_empty() {
                    return Some(ip.to_string());
                }
            }
        }
    }

    // Try X-Real-IP
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(ip) = real_ip.to_str() {
            let ip = ip.trim();
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }

    // Fall back to connection info
    connect_info.map(|addr| addr.ip().to_string())
}

// WebSocket upgrade handler
pub async fn handle_ws_upgrade(
    State(handler): State<Arc<ConnectionHandler>>,
    connect_info: ConnectInfo<SocketAddr>,
    Path(app_key): Path<String>,
    Query(_params): Query<ConnectionQuery>,
    headers: HeaderMap,
    ws: upgrade::IncomingUpgrade,
) -> impl IntoResponse {
    // Extract client IP for rate limiting (from headers or direct connection)
    let client_ip = extract_client_ip(&headers, Some(connect_info.0));

    // Check WebSocket connection rate limit before upgrading
    // Uses increment() to both check and count the connection attempt
    if let Some(rate_limiter) = handler.websocket_rate_limiter() {
        if let Some(ref ip) = client_ip {
            match rate_limiter.increment(ip).await {
                Ok(result) => {
                    if !result.allowed {
                        warn!(
                            "WebSocket connection rate limit exceeded for IP: {} (limit: {}, remaining: {})",
                            ip, result.limit, result.remaining
                        );
                        if let Some(ref metrics) = handler.metrics {
                            let metrics_locked = metrics.lock().await;
                            metrics_locked
                                .mark_connection_error(&app_key, "ws_rate_limit_exceeded");
                        }
                        return (
                            http::StatusCode::TOO_MANY_REQUESTS,
                            "Too many WebSocket connection attempts. Please try again later.",
                        )
                            .into_response();
                    }
                }
                Err(e) => {
                    // Log error but allow connection (fail-open for rate limiting)
                    warn!("WebSocket rate limiter error for IP {}: {}", ip, e);
                }
            }
        }
    }

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
                    | crate::error::Error::OriginNotAllowed
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
