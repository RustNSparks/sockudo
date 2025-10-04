// src/webhook/types.rs
// No SdkConfig needed here, it's for AWS SDK interaction in lambda_sender.
use serde::{Deserialize, Serialize};
use sonic_rs::Value; // Keep this for Value type

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Webhook {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<url::Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function: Option<String>, // Kept for potential legacy or direct Lambda use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda: Option<LambdaConfig>, // For structured Lambda config
    pub event_types: Vec<String>, // Names of events this webhook is interested in
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<WebhookFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<WebhookHeaders>, // Custom headers user might want to add to outgoing webhook
}

// This struct is not directly used in the Pusher payload format,
// but represents the type of events a webhook configuration can subscribe to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEventType {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookFilter {
    pub channel_prefix: Option<String>,
    pub channel_suffix: Option<String>,
    pub channel_pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookHeaders {
    #[serde(flatten)]
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LambdaConfig {
    pub function_name: String,
    pub region: String,
}

// This is the JobData structure that Sockudo uses internally for its queue.
// The `payload` field will be structured to produce the Pusher-compatible format when sent.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JobData {
    pub app_key: String,    // Needed for X-Pusher-Key and signing
    pub app_id: String,     // For logging and potentially for the webhook receiver
    pub app_secret: String, // Needed for signing the X-Pusher-Signature
    pub payload: JobPayload,
    pub original_signature: String, // Sockudo's internal signature for queue deduplication, etc.
}

// This is the JobPayload structure.
// The `events` field will now hold a vector of fully formed Pusher event objects.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JobPayload {
    pub time_ms: i64, // Unix timestamp in milliseconds
    pub events: Vec<Value>, // Each Value is a JSON object representing a Pusher event,
                      // e.g., { "name": "channel_occupied", "channel": "my-channel" }
}

// This struct represents the final payload sent to the webhook receiver,
// aligning with Pusher's format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PusherWebhookPayload {
    pub time_ms: i64,
    pub events: Vec<Value>, // Array of event objects
}
