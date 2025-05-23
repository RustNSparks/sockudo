use crate::adapter::ConnectionHandler;
use crate::app::auth::AuthValidator;
use crate::http_handler::{AppError, EventQuery};
use axum::{
    BoxError,
    body::{Body, Bytes, HttpBody}, // HttpBody and collect are important for body handling
    extract::{FromRequestParts, Request, State}, // Using axum::extract::Request for the whole request
    http::{Method, Request as HttpRequest, StatusCode, Uri, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use serde::de::DeserializeOwned; // For generic JSON payload in original handler
use std::{collections::BTreeMap, sync::Arc};

// Helper to extract query parameters for the signature
fn get_params_for_signature(
    query_str_option: Option<&str>,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut params_map = BTreeMap::new();
    if let Some(query_str) = query_str_option {
        for (key, value) in
            serde_urlencoded::from_str::<Vec<(String, String)>>(query_str).map_err(|e| {
                AppError::InvalidInput(format!(
                    "Failed to parse query string for signature map: {}",
                    e
                ))
            })?
        {
            if key != "auth_signature" {
                params_map.insert(key, value);
            }
        }
    }
    Ok(params_map)
}

/// Axum middleware for Pusher API authentication.
///
/// This middleware authenticates incoming requests based on the Pusher protocol,
/// checking the auth_signature, timestamp, and optionally body_md5.
/// It requires the `ConnectionHandler` state to access the `AppManager` for app details.
pub async fn pusher_api_auth_middleware(
    State(handler_state): State<Arc<ConnectionHandler>>, // Access to AppManager via ConnectionHandler
    request: HttpRequest<Body>,                          // The incoming HTTP request
    next: Next, // The next middleware or handler in the chain
) -> Result<Response, AppError> {
    tracing::debug!("Entering Pusher API Auth Middleware");

    let uri = request.uri().clone();
    let query_str_option = uri.query();
    let method = request.method().clone();
    let path = uri.path().to_string(); // Path is needed for signature

    // 1. Extract Pusher's authentication query parameters (auth_key, auth_timestamp, etc.)
    let auth_q_params_struct: EventQuery = if let Some(query_str) = query_str_option {
        serde_urlencoded::from_str(query_str).map_err(|e| {
            tracing::warn!(
                "Failed to parse EventQuery from query string '{}': {}",
                query_str,
                e
            );
            AppError::InvalidInput(format!("Invalid authentication query parameters: {}", e))
        })?
    } else {
        // Pusher auth requires these parameters. If they are missing, it's an error.
        tracing::warn!("Missing authentication query parameters for Pusher API auth.");
        return Err(AppError::InvalidInput(
            "Missing authentication query parameters".to_string(),
        ));
    };

    // 2. Collect all query parameters (excluding auth_signature) for the signature string.
    let all_query_params_for_sig_map = get_params_for_signature(query_str_option)?;

    // 3. Buffer the request body.
    // The body needs to be read for authentication (e.g., for body_md5 or if the full body is signed)
    // and then made available again for the actual route handler (e.g., for `Json` extraction).
    let (mut parts, body) = request.into_parts();
    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            tracing::error!("Failed to buffer request body for auth: {}", err);
            return Err(AppError::InternalError(format!(
                "Failed to read request body: {}",
                err
            )));
        }
    };
    tracing::debug!("Request body buffered, {} bytes", body_bytes.len());

    // 4. Perform the authentication using AuthValidator.
    let auth_validator = AuthValidator::new(handler_state.app_manager.clone());

    // `validate_pusher_api_request` should return `Result<bool, AppError>` or `Result<(), AppError>`
    // If it returns `Result<bool, AppError>`:
    match auth_validator
        .validate_pusher_api_request(
            &auth_q_params_struct,
            method.as_str(),
            &path, // Pass the cloned path
            &all_query_params_for_sig_map,
            Some(&body_bytes), // Pass the buffered body bytes
        )
        .await
    {
        Ok(true) => {
            tracing::debug!("Pusher API authentication successful for path: {}", path);
            // Auth passed. Reconstruct the request with the buffered body.
            let request = HttpRequest::from_parts(parts, Body::from(body_bytes.clone())); // Use cloned bytes for safety
            Ok(next.run(request).await) // Proceed to the next handler
        }
        Ok(false) => {
            // This case implies validation logic returned `false` without an `Err`.
            tracing::warn!(
                "Pusher API authentication failed (validator returned false) for path: {}",
                path
            );
            Err(AppError::ApiAuthFailed("Invalid API signature".to_string()))
        }
        Err(e) => {
            // If `validate_pusher_api_request` returns an `Err`, it's already an `AppError`.
            tracing::warn!(
                "Pusher API authentication failed (validator returned error) for path: {}: {}",
                path,
                e
            );
            Err(e.into())
        }
    }
}
