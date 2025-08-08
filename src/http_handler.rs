use crate::adapter::ConnectionHandler;
use crate::app::config::App; // To access app limits
use crate::error::{HEALTH_CHECK_TIMEOUT_MS, HealthStatus};
use crate::protocol::constants::EVENT_NAME_MAX_LENGTH as DEFAULT_EVENT_NAME_MAX_LENGTH;
use crate::protocol::messages::{
    ApiMessageData, BatchPusherApiMessage, InfoQueryParser, MessageData, PusherApiMessage,
    PusherMessage,
};
use crate::utils::{self, validate_channel_name};
use crate::websocket::SocketId;
use axum::{
    Json,
    extract::{Path, Query, RawQuery, State}, // Added RawQuery
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header}, // Added Uri
    response::{IntoResponse, Response as AxumResponse},
};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap, // Added BTreeMap
    sync::Arc,
    time::Duration,
};
use sysinfo::System;
use thiserror::Error;
use tokio::time::timeout;
use tracing::{debug, error, field, info, instrument, warn};

// --- Custom Error Type ---

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Application not found: {0}")]
    AppNotFound(String),
    #[error("Application validation failed: {0}")]
    AppValidationFailed(String),
    #[error("API request authentication failed: {0}")]
    ApiAuthFailed(String),
    #[error("Channel validation failed: Missing 'channels' or 'channel' field")]
    MissingChannelInfo,
    #[error("User connection termination failed: {0}")]
    TerminationFailed(String),
    #[error("Internal Server Error: {0}")]
    InternalError(String),
    #[error("Serialization Error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("HTTP Header Build Error: {0}")]
    HeaderBuildError(#[from] axum::http::Error),
    #[error("Limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> AxumResponse {
        let (status, error_message) = match &self {
            AppError::AppNotFound(msg) => (StatusCode::NOT_FOUND, json!({ "error": msg })),
            AppError::AppValidationFailed(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": msg }))
            }
            AppError::ApiAuthFailed(msg) => (StatusCode::FORBIDDEN, json!({ "error": msg })),
            AppError::MissingChannelInfo => (
                StatusCode::BAD_REQUEST,
                json!({ "error": "Request must contain 'channels' (list) or 'channel' (string)" }),
            ),
            AppError::TerminationFailed(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": msg }))
            }
            AppError::SerializationError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("Internal error during serialization: {}", e) }),
            ),
            AppError::HeaderBuildError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("Internal error building response: {}", e) }),
            ),
            AppError::InternalError(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": msg }))
            }
            AppError::LimitExceeded(msg) => (StatusCode::BAD_REQUEST, json!({ "error": msg })),
            AppError::InvalidInput(msg) => (StatusCode::BAD_REQUEST, json!({ "error": msg })),
        };
        error!(error.message = %self, status_code = %status, "HTTP request failed");
        (status, Json(error_message)).into_response()
    }
}

impl From<crate::error::Error> for AppError {
    fn from(err: crate::error::Error) -> Self {
        warn!(original_error = ?err, "Converting internal error to AppError for HTTP response");
        match err {
            crate::error::Error::InvalidAppKey => {
                AppError::AppNotFound(format!("Application key not found or invalid: {err}"))
            }
            crate::error::Error::ApplicationNotFound => AppError::AppNotFound(err.to_string()),
            crate::error::Error::InvalidChannelName(s) => {
                AppError::InvalidInput(format!("Invalid channel name: {s}"))
            }
            crate::error::Error::Channel(s) => AppError::InvalidInput(s),
            crate::error::Error::Auth(s) => AppError::ApiAuthFailed(s),
            _ => AppError::InternalError(err.to_string()),
        }
    }
}

// --- Query Parameter Structs ---

// This struct is used by Axum to deserialize the known Pusher auth query parameters.
// It's also passed to `validate_pusher_api_request` to easily access these specific values.
#[derive(Deserialize, Debug, Clone)]
pub struct EventQuery {
    #[serde(default)]
    pub auth_key: String,
    #[serde(default)]
    pub auth_timestamp: String,
    #[serde(default)]
    pub auth_version: String,
    #[serde(default)]
    pub body_md5: String,
    #[serde(default)]
    pub auth_signature: String,
}

#[derive(Deserialize, Debug)]
pub struct ChannelQuery {
    #[serde(default)]
    pub info: Option<String>,
    // EventQuery fields are flattened here for GET requests that also need specific endpoint params
    #[serde(flatten)]
    pub auth_params: EventQuery,
}

#[derive(Deserialize, Debug)]
pub struct ChannelsQuery {
    #[serde(default)]
    pub filter_by_prefix: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
    // EventQuery fields are flattened here
    #[serde(flatten)]
    pub auth_params: EventQuery,
}

// --- Response Structs ---

#[derive(Serialize)]
struct MemoryStats {
    free: u64,
    used: u64,
    total: u64,
    percent: f64,
}

#[derive(Serialize)]
struct UsageResponse {
    memory: MemoryStats,
}

// --- Helper Functions ---

/// Helper to build cache payload string
fn build_cache_payload(event_name: &str, event_data: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&json!({
        "event": event_name,
        "data": event_data,
    }))
}

/// Records API metrics (helper async function)
#[instrument(skip(handler, incoming_request_size, outgoing_response_size), fields(app_id = %app_id))]
async fn record_api_metrics(
    handler: &Arc<ConnectionHandler>,
    app_id: &str,
    incoming_request_size: usize,
    outgoing_response_size: usize,
) {
    if let Some(metrics_arc) = &handler.metrics {
        let metrics = metrics_arc.lock().await;
        metrics.mark_api_message(app_id, incoming_request_size, outgoing_response_size);
        debug!(
            incoming_bytes = incoming_request_size,
            outgoing_bytes = outgoing_response_size,
            "Recorded API message metrics"
        );
    } else {
        debug!(
            "{}",
            "Metrics system not available, skipping metrics recording."
        );
    }
}

// --- API Handlers ---

/// GET /usage
#[instrument(skip_all, fields(service = "usage_monitor"))]
pub async fn usage() -> Result<impl IntoResponse, AppError> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let total = sys.total_memory() * 1024;
    let used = sys.used_memory() * 1024;
    let free = total.saturating_sub(used);
    let percent = if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let memory_stats = MemoryStats {
        free,
        used,
        total,
        percent,
    };
    let response_payload = UsageResponse {
        memory: memory_stats,
    };

    info!(
        total_bytes = total,
        used_bytes = used,
        free_bytes = free,
        usage_percent = format!("{:.2}", percent),
        "Memory usage queried"
    );

    Ok((StatusCode::OK, Json(response_payload)))
}

/// Helper to process a single event and return channel info if requested
#[instrument(skip(handler, event_data, app), fields(app_id = app.id, event_name = field::Empty))]
async fn process_single_event_parallel(
    handler: &Arc<ConnectionHandler>,
    app: &App,
    event_data: PusherApiMessage,
    collect_info: bool,
) -> Result<HashMap<String, Value>, AppError> {
    // Destructure the incoming event data
    let PusherApiMessage {
        name,
        data: event_payload_data, // This is Option<ApiMessageData>
        channels,
        channel,
        socket_id: original_socket_id_str, // Option<String>
        info,                              // Option<String>
    } = event_data;

    // Validate and get the event name
    let event_name_str = name
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("Event name is required".to_string()))?;
    tracing::Span::current().record("event_name", event_name_str);

    // Check event name length against app limits
    let max_event_name_len = app
        .max_event_name_length
        .unwrap_or(DEFAULT_EVENT_NAME_MAX_LENGTH as u32);
    if event_name_str.len() > max_event_name_len as usize {
        return Err(AppError::LimitExceeded(format!(
            "Event name '{event_name_str}' exceeds maximum length of {max_event_name_len}"
        )));
    }

    // Check event payload size against app limits
    if let Some(max_payload_kb) = app.max_event_payload_in_kb {
        let value_for_size_calc = match &event_payload_data {
            Some(ApiMessageData::String(s)) => json!(s),
            Some(ApiMessageData::Json(j_val)) => j_val.clone(),
            None => json!(null),
        };
        let payload_size_bytes = utils::data_to_bytes_flexible(vec![value_for_size_calc]);
        if payload_size_bytes > (max_payload_kb as usize * 1024) {
            return Err(AppError::LimitExceeded(format!(
                "Event payload size ({payload_size_bytes} bytes) for event '{event_name_str}' exceeds limit ({max_payload_kb}KB)"
            )));
        }
    }

    // Map the original socket ID string to SocketId type
    let mapped_socket_id: Option<SocketId> = original_socket_id_str.map(SocketId);

    // Determine the list of target channels for this event
    let target_channels: Vec<String> = match channels {
        Some(ch_list) if !ch_list.is_empty() => {
            if let Some(max_ch_at_once) = app.max_event_channels_at_once
                && ch_list.len() > max_ch_at_once as usize
            {
                return Err(AppError::LimitExceeded(format!(
                    "Number of channels ({}) exceeds limit ({})",
                    ch_list.len(),
                    max_ch_at_once
                )));
            }
            ch_list
        }
        None => match channel {
            Some(ch_str) => vec![ch_str],
            None => {
                warn!("{}", "Missing 'channels' or 'channel' in event");
                return Err(AppError::MissingChannelInfo);
            }
        },
        Some(_) => {
            warn!("{}", "Empty 'channels' list provided in event");
            return Err(AppError::MissingChannelInfo);
        }
    };

    // Create a collection of futures, one for each channel to process.
    // These futures will be executed concurrently by `join_all`.
    let channel_processing_futures = target_channels.into_iter().map(|target_channel_str| {
        // Clone data needed for each concurrent task.
        // Arc clones are cheap. Owned data (String, Option<String>, etc.) is cloned.
        let handler_clone = Arc::clone(handler);
        // `app` is a reference (&App), it will be captured by the async block.
        // This is safe as `join_all` awaits futures within `app`'s lifetime.
        let name_for_task = name.clone(); // Option<String>
        let payload_for_task = event_payload_data.clone(); // Option<ApiMessageData>
        let socket_id_for_task = mapped_socket_id.clone(); // Option<SocketId>
        let info_for_task = info.clone(); // Option<String>
        let event_name_for_task = event_name_str.to_string(); // String

        async move {
            // This block processes a single channel.
            debug!(channel = %target_channel_str, "Processing channel for event (parallel task)");

            // Validate the channel name.
            // `app` is captured by reference from the outer scope.
            validate_channel_name(app, &target_channel_str).await?;

            // Construct the message to be sent to this specific channel.
            let message_data = match payload_for_task {
                Some(ApiMessageData::String(s)) => MessageData::String(s),
                Some(ApiMessageData::Json(j_val)) => MessageData::Json(j_val),
                None => MessageData::Json(json!(null)), // Default to null if no data
            };
            let _message_to_send = PusherMessage {
                channel: Some(target_channel_str.clone()),
                name: None,
                event: name_for_task,
                data: Some(message_data.clone()),
            };
            handler_clone.broadcast_to_channel(app, &target_channel_str, _message_to_send, socket_id_for_task.as_ref())
                .await?;


            // If info collection is requested, gather details for this channel.
            let mut collected_channel_specific_info: Option<(String, Value)> = None;
            if collect_info {
                let is_presence = target_channel_str.starts_with("presence-");
                let mut current_channel_info_map = serde_json::Map::new();

                // Get user count for presence channels if requested.
                if is_presence && info_for_task.as_deref().is_some_and(|s| s.contains("user_count")) {
                    match handler_clone
                        .channel_manager
                        .read()
                        .await
                        .get_channel_members(&app.id, &target_channel_str)
                        .await
                    {
                        Ok(members_map) => {
                            current_channel_info_map
                                .insert("user_count".to_string(), json!(members_map.len()));
                        }
                        Err(e) => {
                            warn!(
                                "Failed to get user count for channel {}: {} (internal error: {:?})",
                                target_channel_str, e, e
                            );
                        }
                    }
                }

                // Get subscription count if requested.
            if info_for_task
                .as_deref()
                .is_some_and(|s| s.contains("subscription_count"))
                {
                    let count = handler_clone
                        .connection_manager
                        .lock()
                        .await
                        .get_channel_socket_count(&app.id, &target_channel_str)
                        .await;
                    current_channel_info_map.insert("subscription_count".to_string(), json!(count));
                }

                if !current_channel_info_map.is_empty() {
                    collected_channel_specific_info = Some((
                        target_channel_str.clone(),
                        Value::Object(current_channel_info_map),
                    ));
                }
            }

            // Handle caching for cacheable channels.
            if utils::is_cache_channel(&target_channel_str) {
                // // Use the payload cloned for the task (payload_for_task)
                // let payload_value_for_cache = match payload_for_task {
                //     Some(ApiMessageData::String(s)) => json!(s),
                //     Some(ApiMessageData::Json(j_val)) => j_val, // Already a Value
                //     None => json!(null),
                // };
                let message_data = serde_json::to_value(&message_data)
                    .map_err(AppError::SerializationError)?;
                // Attempt to build the cache payload string.
                match build_cache_payload(&event_name_for_task, &message_data) {
                    Ok(cache_payload_str) => {
                        let mut cache_manager_locked = handler_clone.cache_manager.lock().await;
                        let cache_key_str =
                            format!("app:{}:channel:{}:cache_miss", &app.id, target_channel_str);

                        // Attempt to set the cache entry.
                        match cache_manager_locked
                            .set(&cache_key_str, &cache_payload_str, 3600) // TTL is 1 hour
                            .await
                        {
                            Ok(_) => {
                                debug!(channel = %target_channel_str, cache_key = %cache_key_str, "Cached event for channel");
                            }
                            Err(e) => {
                                error!(channel = %target_channel_str, cache_key = %cache_key_str, error = %e, "Failed to cache event (internal error: {:?})", e);
                            }
                        }
                    }
                    Err(e) => {
                        // Log error and continue without caching, task itself doesn't fail here.
                        error!(channel = %target_channel_str, error = %e, "Failed to serialize event data for caching");
                    }
                }
            }
            // Return the collected info for this channel, or None if no info was collected.
            // The overall Result wraps this for error propagation from validation, etc.
            Ok(collected_channel_specific_info)
        }
    });

    // Execute all channel processing futures concurrently.
    let results: Vec<Result<Option<(String, Value)>, AppError>> =
        join_all(channel_processing_futures).await;

    // Aggregate results from all tasks.
    let mut final_channels_info_map = HashMap::new();
    for result in results {
        match result {
            Ok(Some((channel_name, info_value))) => {
                // If info was collected for a channel, add it to the map.
                final_channels_info_map.insert(channel_name, info_value);
            }
            Ok(None) => {
                // No info collected for this channel, or info collection was not requested.
            }
            Err(e) => {
                // If any task failed (e.g., validation error), propagate the error immediately.
                // This maintains the original function's fail-fast behavior.
                return Err(e);
            }
        }
    }

    // Return the aggregated map of channel information.
    Ok(final_channels_info_map)
}

/// POST /apps/{app_id}/events
#[instrument(skip(handler, event_payload), fields(app_id = %app_id))]
pub async fn events(
    Path(app_id): Path<String>,
    Query(auth_q_params_struct): Query<EventQuery>, // Axum deserializes known auth params here
    State(handler): State<Arc<ConnectionHandler>>,
    uri: Uri, // To get the request path (e.g., "/apps/app_id_123/events")
    RawQuery(raw_query_str_option): RawQuery, // Gets the raw query string (e.g., "auth_key=abc&auth_timestamp=123...")
    Json(event_payload): Json<PusherApiMessage>, // The JSON body of the request
) -> Result<impl IntoResponse, AppError> {
    // Calculate request size for metrics
    let incoming_request_size_bytes = serde_json::to_vec(&event_payload)?.len();

    let app = handler
        .app_manager
        .find_by_id(app_id.as_str())
        .await?
        .ok_or_else(|| AppError::AppNotFound(app_id.clone()))?;

    let need_channel_info = event_payload.info.is_some();

    let channels_info_map =
        process_single_event_parallel(&handler, &app, event_payload, need_channel_info).await?;

    let response_payload = if need_channel_info && !channels_info_map.is_empty() {
        json!({
            "channels": channels_info_map
        })
    } else {
        json!({ "ok": true })
    };

    // Calculate response size for metrics and record metrics
    let outgoing_response_size_bytes = serde_json::to_vec(&response_payload)?.len();
    record_api_metrics(
        &handler,
        &app_id,
        incoming_request_size_bytes,
        outgoing_response_size_bytes,
    )
    .await;

    Ok((StatusCode::OK, Json(response_payload)))
}

/// POST /apps/{app_id}/batch_events
#[instrument(skip_all, fields(app_id = %app_id, batch_len = field::Empty))]
pub async fn batch_events(
    Path(app_id): Path<String>,
    Query(_auth_q_params_struct): Query<EventQuery>,
    State(handler): State<Arc<ConnectionHandler>>,
    _uri: Uri,
    RawQuery(_raw_query_str_option): RawQuery,
    Json(batch_message_payload): Json<BatchPusherApiMessage>,
) -> Result<impl IntoResponse, AppError> {
    let body_bytes = serde_json::to_vec(&batch_message_payload)?;
    let batch_events_vec = batch_message_payload.batch;
    let batch_len = batch_events_vec.len();
    tracing::Span::current().record("batch_len", batch_len);
    debug!("Received batch events request with {} events", batch_len);

    // Fetch app configuration once.
    let app_config = handler
        .app_manager
        .find_by_id(app_id.as_str())
        .await?
        .ok_or_else(|| AppError::AppNotFound(app_id.clone()))?;

    // Validate batch size against app limits.
    if let Some(max_batch) = app_config.max_event_batch_size
        && batch_len > max_batch as usize
    {
        return Err(AppError::LimitExceeded(format!(
            "Batch size ({batch_len}) exceeds limit ({max_batch})"
        )));
    }

    let incoming_request_size_bytes = body_bytes.len(); // Use length of already serialized body_bytes
    let mut any_message_requests_info = false;

    // Determine if any event in the batch requests info.
    // This needs to be done before creating tasks if tasks are to conditionally collect info.
    // However, process_single_event_parallel already takes a `collect_info` boolean.
    // We can pass this boolean per task.
    for single_event_message in &batch_events_vec {
        // Iterate by reference first
        if single_event_message.info.is_some() {
            any_message_requests_info = true;
            break;
        }
    }

    // Create a collection of futures for processing each event in the batch.
    let event_processing_futures = batch_events_vec.into_iter().map(|single_event_message| {
        // Clone Arcs and capture references/owned data for the async task.
        let handler_clone = Arc::clone(&handler);
        let app_config_ref = &app_config; // Reference to app_config owned by batch_events
        // single_event_message is moved into this closure, then cloned for process_single_event_parallel

        async move {
            let should_collect_info_for_this_event = single_event_message.info.is_some();
            let channel_info_map = process_single_event_parallel(
                &handler_clone,
                app_config_ref,
                single_event_message.clone(), // Clone for process_single_event_parallel
                should_collect_info_for_this_event,
            )
            .await?;
            // Return the original message (for constructing response) and the processed info map
            Ok((single_event_message, channel_info_map))
        }
    });

    // Execute all event processing futures concurrently.
    type EventResult = Result<(PusherApiMessage, HashMap<String, Value>), AppError>;

    let results: Vec<EventResult> = join_all(event_processing_futures).await;

    // Aggregate results and construct the batch response.
    let mut batch_response_info_vec = Vec::with_capacity(batch_len);
    let mut processed_event_data = Vec::with_capacity(batch_len);

    for result_item in results {
        processed_event_data.push(result_item?); // Propagate the first error encountered.
    }

    // Now, construct the response based on the successfully processed events.
    if any_message_requests_info {
        for (original_msg, channel_info_map_for_event) in processed_event_data {
            // This logic for extracting main_channel_for_event and info is from the original loop.
            if let Some(main_channel_for_event) = original_msg
                .channel
                .as_ref()
                .or_else(|| original_msg.channels.as_ref().and_then(|chs| chs.first()))
            {
                batch_response_info_vec.push(
                    channel_info_map_for_event
                        .get(main_channel_for_event)
                        .cloned()
                        .unwrap_or_else(|| json!({})), // Default to empty object if no specific info
                );
            } else {
                // If no main channel could be determined (should be rare if validation passed),
                // push an empty object.
                batch_response_info_vec.push(json!({}));
            }
        }
    }

    // Construct the final JSON response payload.
    let final_response_payload = if any_message_requests_info {
        json!({ "batch": batch_response_info_vec })
    } else {
        json!({}) // Pusher API expects an empty JSON object {} for success if no info requested.
    };

    // Record metrics and return the response.
    let outgoing_response_size_bytes_vec = serde_json::to_vec(&final_response_payload)?;
    record_api_metrics(
        &handler,
        &app_id,
        incoming_request_size_bytes,
        outgoing_response_size_bytes_vec.len(),
    )
    .await;
    debug!("{}", "Batch events processed successfully");
    Ok((StatusCode::OK, Json(final_response_payload)))
}

/// GET /apps/{app_id}/channels/{channel_name}
#[instrument(skip(handler), fields(app_id = %app_id, channel = %channel_name))]
pub async fn channel(
    Path((app_id, channel_name)): Path<(String, String)>,
    Query(query_params_specific): Query<ChannelQuery>, // Contains specific params like 'info' AND flattened auth_params
    State(handler): State<Arc<ConnectionHandler>>,
    uri: Uri,
    RawQuery(raw_query_str_option): RawQuery,
) -> Result<impl IntoResponse, AppError> {
    debug!("Request for channel info for channel: {}", channel_name);
    let app = handler
        .app_manager
        .find_by_id(&app_id)
        .await?
        .ok_or_else(|| AppError::AppNotFound(app_id.clone()))?;

    validate_channel_name(&app, &channel_name).await?;

    let info_query_str = query_params_specific.info.as_ref(); // Use specific params from ChannelQuery
    let wants_subscription_count = info_query_str.wants_subscription_count();
    let wants_user_count = info_query_str.wants_user_count();
    let wants_cache_data = info_query_str.wants_cache();

    let socket_count_val;
    {
        let mut connection_manager_locked = handler.connection_manager.lock().await;
        socket_count_val = connection_manager_locked
            .get_channel_socket_count(&app_id, &channel_name)
            .await;
    }

    let user_count_val = if wants_user_count {
        if channel_name.starts_with("presence-") {
            let members_map = handler
                .channel_manager
                .read()
                .await
                .get_channel_members(&app_id, &channel_name)
                .await?;
            Some(members_map.len() as u64)
        } else {
            return Err(AppError::InvalidInput(
                "user_count is only available for presence channels".to_string(),
            ));
        }
    } else {
        None
    };

    let cache_data_tuple = if wants_cache_data && utils::is_cache_channel(&channel_name) {
        let mut cache_manager_locked = handler.cache_manager.lock().await;
        let cache_key_str = format!("app:{app_id}:channel:{channel_name}:cache_miss");

        match cache_manager_locked.get(&cache_key_str).await? {
            Some(cache_content_str) => {
                let ttl_duration = cache_manager_locked
                    .ttl(&cache_key_str)
                    .await?
                    .unwrap_or_else(|| core::time::Duration::from_secs(3600));
                Some((cache_content_str, ttl_duration))
            }
            _ => None,
        }
    } else {
        None
    };

    let subscription_count_val = if wants_subscription_count {
        Some(socket_count_val as u64)
    } else {
        None
    };
    let response_payload = PusherMessage::channel_info(
        socket_count_val > 0,
        subscription_count_val,
        user_count_val,
        cache_data_tuple,
    );
    let response_json_bytes = serde_json::to_vec(&response_payload)?;
    record_api_metrics(&handler, &app_id, 0, response_json_bytes.len()).await;
    debug!("Channel info for '{}' retrieved successfully", channel_name);
    Ok((StatusCode::OK, Json(response_payload)))
}

/// GET /apps/{app_id}/channels
#[instrument(skip(handler), fields(app_id = %app_id))]
pub async fn channels(
    Path(app_id): Path<String>,
    Query(query_params_specific): Query<ChannelsQuery>, // Contains specific params AND flattened auth_params
    State(handler): State<Arc<ConnectionHandler>>,
    uri: Uri,
    RawQuery(raw_query_str_option): RawQuery,
) -> Result<impl IntoResponse, AppError> {
    debug!("Request for channels list for app_id: {}", app_id);

    let filter_prefix_str = query_params_specific
        .filter_by_prefix
        .as_deref()
        .unwrap_or("");
    let wants_user_count = query_params_specific.info.as_ref().wants_user_count();
    let app = handler
        .app_manager
        .find_by_id(&app_id)
        .await?
        .ok_or_else(|| AppError::AppNotFound(app_id.clone()))?;

    let channels_map;
    {
        let mut connection_manager_locked = handler.connection_manager.lock().await;
        channels_map = connection_manager_locked
            .get_channels_with_socket_count(&app_id)
            .await?;
    }

    let mut channels_info_response_map = HashMap::new();
    for entry in channels_map.iter() {
        let channel_name_str = entry.key();
        if !channel_name_str.starts_with(filter_prefix_str) {
            continue;
        }
        validate_channel_name(&app, channel_name_str).await?;
        let mut current_channel_info_map = serde_json::Map::new();
        if wants_user_count {
            if channel_name_str.starts_with("presence-") {
                let members_map = handler
                    .channel_manager
                    .read()
                    .await
                    .get_channel_members(&app_id, channel_name_str)
                    .await?;
                current_channel_info_map.insert("user_count".to_string(), json!(members_map.len()));
            } else if !filter_prefix_str.starts_with("presence-") {
                return Err(AppError::InvalidInput(
                    "user_count is only available for presence channels. Use filter_by_prefix=presence-".to_string()
                ));
            }
        }
        if !current_channel_info_map.is_empty() {
            channels_info_response_map.insert(
                channel_name_str.clone(),
                Value::Object(current_channel_info_map),
            );
        } else if query_params_specific.info.is_none() {
            channels_info_response_map.insert(channel_name_str.clone(), json!({}));
        }
    }

    let response_payload = PusherMessage::channels_list(channels_info_response_map);
    let response_json_bytes = serde_json::to_vec(&response_payload)?;
    record_api_metrics(&handler, &app_id, 0, response_json_bytes.len()).await;
    debug!("Channels list for app '{}' retrieved successfully", app_id);
    Ok((StatusCode::OK, Json(response_payload)))
}

/// GET /apps/{app_id}/channels/{channel_name}/users
#[instrument(skip(handler), fields(app_id = %app_id, channel = %channel_name))]
pub async fn channel_users(
    Path((app_id, channel_name)): Path<(String, String)>,
    Query(auth_q_params_struct): Query<EventQuery>, // Only auth params for this GET endpoint
    State(handler): State<Arc<ConnectionHandler>>,
) -> Result<impl IntoResponse, AppError> {
    let app = handler
        .app_manager
        .find_by_id(&app_id)
        .await?
        .ok_or_else(|| AppError::AppNotFound(app_id.clone()))?;
    debug!("Request for users in channel: {}", channel_name);
    validate_channel_name(&app, &channel_name).await?;

    if !channel_name.starts_with("presence-") {
        return Err(AppError::InvalidInput(
            "Only presence channels support this endpoint".to_string(),
        ));
    }

    let channel_members_map = handler
        .channel_manager
        .read()
        .await
        .get_channel_members(&app_id, &channel_name)
        .await?;
    let users_vec = channel_members_map
        .keys()
        .map(|user_id_str| json!({ "id": user_id_str }))
        .collect::<Vec<_>>();
    let response_payload_val = json!({ "users": users_vec });
    let response_json_bytes = serde_json::to_vec(&response_payload_val)?;
    record_api_metrics(&handler, &app_id, 0, response_json_bytes.len()).await;
    info!(
        user_count = users_vec.len(),
        "Channel users for '{}' retrieved successfully", channel_name
    );
    Ok((StatusCode::OK, Json(response_payload_val)))
}

/// POST /apps/{app_id}/users/{user_id}/terminate_connections
#[instrument(skip(handler), fields(app_id = %app_id, user_id = %user_id))]
pub async fn terminate_user_connections(
    Path((app_id, user_id)): Path<(String, String)>,
    Query(auth_q_params_struct): Query<EventQuery>, // Auth params for this POST endpoint
    State(handler): State<Arc<ConnectionHandler>>,
    uri: Uri,
    RawQuery(raw_query_str_option): RawQuery,
    // This endpoint typically does not have a body, but if it could, it would be:
    // axum::body::Bytes(body_bytes_payload): axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    info!(
        "Received request to terminate user connections for user_id: {}",
        user_id
    );

    let connection_manager_arc = handler.connection_manager.clone();
    connection_manager_arc
        .lock()
        .await
        .terminate_connection(&app_id, &user_id)
        .await?;

    info!(
        "Successfully initiated termination for user_id: {}",
        user_id
    );

    let response_payload = json!({ "ok": true });
    let response_size = serde_json::to_vec(&response_payload)?.len();
    record_api_metrics(&handler, &app_id, 0, response_size).await;

    Ok((StatusCode::OK, Json(response_payload)))
}

/// System health check function - performs comprehensive checks
/// Uses centralized timeouts to prevent hanging and avoid deadlocks
/// All individual health checks are wrapped with HEALTH_CHECK_TIMEOUT_MS
///
/// Critical components (WebSocket functionality fails without these):
/// - Adapter: Core WebSocket connection handling
/// - Cache Manager: Required for cache channels, subscription failures propagate errors
///
/// Non-critical components (WebSocket works, but optional features don't):
/// - Queue System: Only affects webhook delivery
async fn check_system_health(handler: &Arc<ConnectionHandler>) -> HealthStatus {
    let mut critical_issues = Vec::new();
    let mut non_critical_issues = Vec::new();

    // CRITICAL CHECK 1: Adapter health - core WebSocket functionality
    let adapter_check = timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS), async {
        let conn_mgr = handler.connection_manager.lock().await;
        conn_mgr.check_health().await
    })
    .await;

    match adapter_check {
        Ok(Ok(())) => {
            // Adapter is healthy
        }
        Ok(Err(e)) => {
            critical_issues.push(format!("Adapter: {e}"));
        }
        Err(_) => {
            critical_issues.push("Adapter health check timeout".to_string());
        }
    }

    // CRITICAL CHECK 2: Cache manager health - required for cache channels
    if handler.server_options().cache.driver != crate::options::CacheDriver::None {
        let cache_check = timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS), async {
            let cache_manager = handler.cache_manager.lock().await;
            cache_manager.check_health().await
        })
        .await;

        match cache_check {
            Ok(Ok(())) => {
                // Cache manager is healthy
            }
            Ok(Err(e)) => {
                critical_issues.push(format!("Cache: {e}"));
            }
            Err(_) => {
                critical_issues.push("Cache health check timeout".to_string());
            }
        }
    }

    // NON-CRITICAL CHECK: Queue system health - only affects webhooks
    if let Some(webhook_integration) = handler.webhook_integration() {
        let queue_check = timeout(
            Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS),
            webhook_integration.check_queue_health(),
        )
        .await;

        match queue_check {
            Ok(Ok(())) => {
                // Queue is healthy
            }
            Ok(Err(e)) => {
                non_critical_issues.push(format!("Webhooks: {e}"));
            }
            Err(_) => {
                non_critical_issues.push("Webhook queue health check timeout".to_string());
            }
        }
    }

    // Return appropriate status based on critical vs non-critical failures
    if !critical_issues.is_empty() {
        // Critical component failure = Error (service unavailable)
        HealthStatus::Error(critical_issues)
    } else if !non_critical_issues.is_empty() {
        // Only non-critical failures = Degraded (service works but some features don't)
        HealthStatus::Degraded(non_critical_issues)
    } else {
        // All components healthy
        HealthStatus::Ok
    }
}

/// GET /up or /up/{app_id}
/// When app_id is provided, checks specific app id health and general system health
/// When no app_id, checks general system health and ensures at least 1 app exists
#[instrument(skip(handler), fields(app_id = field::Empty))]
pub async fn up(
    app_id: Option<Path<String>>,
    State(handler): State<Arc<ConnectionHandler>>,
) -> Result<impl IntoResponse, AppError> {
    // First check app configuration based on whether app_id is provided
    let (health_status, app_id_str) = if let Some(Path(app_id)) = app_id {
        // Specific app health check
        tracing::Span::current().record("app_id", &app_id);
        debug!("Health check received for app_id: {}", app_id);

        // Check if specific app exists and is enabled
        let app_check = timeout(
            Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS),
            handler.app_manager.find_by_id(&app_id),
        )
        .await;

        let app_status = match app_check {
            Ok(Ok(Some(app))) if app.enabled => {
                // App exists and is enabled, proceed with system checks
                check_system_health(&handler).await
            }
            Ok(Ok(Some(_))) => {
                // App exists but is disabled
                HealthStatus::Error(vec!["App is disabled".to_string()])
            }
            Ok(Ok(None)) => {
                // App doesn't exist
                HealthStatus::NotFound
            }
            Ok(Err(e)) => {
                // App manager error
                HealthStatus::Error(vec![format!("App manager: {e}")])
            }
            Err(_) => {
                // Timeout
                HealthStatus::Error(vec![format!(
                    "App manager timeout (>{HEALTH_CHECK_TIMEOUT_MS}ms)"
                )])
            }
        };

        (app_status, app_id)
    } else {
        // General system health check
        debug!("General health check received (no app_id)");

        // Check if at least one app is configured
        let apps_check = timeout(
            Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS),
            handler.app_manager.get_apps(),
        )
        .await;

        let app_status = match apps_check {
            Ok(Ok(apps)) if !apps.is_empty() => {
                debug!("Found {} configured apps", apps.len());
                // At least one app exists, proceed with system checks
                check_system_health(&handler).await
            }
            Ok(Ok(_)) => {
                // No apps configured
                HealthStatus::Error(vec!["No apps configured".to_string()])
            }
            Ok(Err(e)) => {
                // App manager error
                HealthStatus::Error(vec![format!("App manager: {e}")])
            }
            Err(_) => {
                // Timeout
                HealthStatus::Error(vec![format!(
                    "App manager timeout (>{HEALTH_CHECK_TIMEOUT_MS}ms)"
                )])
            }
        };

        (app_status, "system".to_string())
    };

    // Log detailed issues if unhealthy
    match &health_status {
        HealthStatus::Ok => {
            debug!("Health check passed for {}", app_id_str);
        }
        HealthStatus::Degraded(reasons) => {
            warn!(
                "Health check degraded for {}: {}",
                app_id_str,
                reasons.join(", ")
            );
        }
        HealthStatus::Error(reasons) => {
            error!(
                "Health check failed for {}: {}",
                app_id_str,
                reasons.join(", ")
            );
        }
        HealthStatus::NotFound => {
            warn!("Health check for non-existent app_id: {}", app_id_str);
        }
    }

    // Return response maintaining backward compatibility
    // Critical component failures return 503, non-critical failures return 200
    let (status_code, status_text, header_value) = match health_status {
        HealthStatus::Ok => (StatusCode::OK, "OK", "OK"),
        HealthStatus::Degraded(_) => (StatusCode::OK, "DEGRADED", "DEGRADED"), // Non-critical issues - WebSocket still works
        HealthStatus::Error(_) => (StatusCode::SERVICE_UNAVAILABLE, "ERROR", "ERROR"), // Critical issues - WebSocket won't work
        HealthStatus::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "NOT_FOUND"),
    };

    // Record metrics if available (non-blocking)
    if handler.metrics.is_some() {
        let response_size = status_text.len();
        record_api_metrics(&handler, &app_id_str, 0, response_size).await;
    }

    // Build response using the original format
    let response_val = axum::http::Response::builder()
        .status(status_code)
        .header("X-Health-Check", header_value)
        .body(status_text.to_string())?;

    Ok(response_val)
}

/// GET /metrics (Prometheus format)
#[instrument(skip(handler), fields(service = "metrics_exporter"))]
pub async fn metrics(
    State(handler): State<Arc<ConnectionHandler>>,
) -> Result<impl IntoResponse, AppError> {
    debug!("{}", "Metrics endpoint called");
    let plaintext_metrics_str = match handler.metrics.clone() {
        Some(metrics_arc) => {
            let metrics_data_guard = metrics_arc.lock().await;
            metrics_data_guard.get_metrics_as_plaintext().await
        }
        None => {
            info!(
                "{}",
                "No metrics data available (metrics collection is not enabled)."
            );
            "# Metrics collection is not enabled.\n".to_string()
        }
    };
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    debug!(
        bytes = plaintext_metrics_str.len(),
        "Successfully generated Prometheus metrics"
    );
    Ok((StatusCode::OK, response_headers, plaintext_metrics_str))
}
