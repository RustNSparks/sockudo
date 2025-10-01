use crate::adapter::horizontal_adapter::{
    BroadcastMessage, RequestBody, RequestType, ResponseBody,
};
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};
use bincode::Options;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Protocol version for backward compatibility during rolling upgrades
pub const BINARY_PROTOCOL_VERSION: u8 = 1;

/// Maximum message size (10MB) to prevent DoS attacks
pub const MAX_MESSAGE_SIZE: u64 = 10 * 1024 * 1024;

/// Get the bincode configuration for consistent serialization
/// Uses DefaultOptions with a size limit for safety
pub fn bincode_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_limit(MAX_MESSAGE_SIZE)
        .with_little_endian()
        .with_varint_encoding()
}

/// Binary envelope for broadcast messages
/// Wraps the original JSON client payload without re-parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryBroadcastMessage {
    /// Protocol version for backward compatibility
    pub version: u8,

    /// Hash of channel name for fast routing (xxh3_64)
    pub channel_hash: u64,

    /// Channel name (kept for routing, but we also use hash)
    pub channel: String,

    /// Node ID as fixed-size bytes for efficiency
    pub node_id_bytes: [u8; 16], // UUID is 16 bytes

    /// App ID (keeping as string since it's user-defined)
    pub app_id: String,

    /// Raw client JSON message bytes (not re-parsed)
    /// This is the exact JSON that will be sent to clients
    pub raw_client_json: Vec<u8>,

    /// Optional socket ID to exclude from broadcast
    pub except_socket_id: Option<String>,

    /// Timestamp in milliseconds since epoch (microsecond precision as f64)
    pub timestamp_ms: Option<f64>,
}

/// Binary envelope for request messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryRequestBody {
    /// Protocol version
    pub version: u8,

    /// Request ID as fixed-size bytes
    pub request_id_bytes: [u8; 16],

    /// Node ID as fixed-size bytes
    pub node_id_bytes: [u8; 16],

    /// App ID
    pub app_id: String,

    /// Request type (enum serialized efficiently)
    pub request_type: RequestType,

    /// Channel name (optional)
    pub channel: Option<String>,

    /// Channel hash (optional, for fast routing)
    pub channel_hash: Option<u64>,

    /// Socket ID (optional)
    pub socket_id: Option<String>,

    /// User ID (optional)
    pub user_id: Option<String>,

    /// Serialized user info (as bytes instead of JSON Value)
    pub user_info_bytes: Option<Vec<u8>>,

    /// Timestamp for heartbeat
    pub timestamp: Option<u64>,

    /// Dead node ID (optional)
    pub dead_node_id: Option<String>,

    /// Target node ID (optional)
    pub target_node_id: Option<String>,
}

/// Binary envelope for response messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryResponseBody {
    /// Protocol version
    pub version: u8,

    /// Request ID as fixed-size bytes
    pub request_id_bytes: [u8; 16],

    /// Node ID as fixed-size bytes
    pub node_id_bytes: [u8; 16],

    /// App ID
    pub app_id: String,

    /// Serialized members map (as bytes)
    pub members_bytes: Option<Vec<u8>>,

    /// Serialized channels with socket count (as bytes)
    pub channels_with_sockets_count_bytes: Option<Vec<u8>>,

    /// Socket IDs
    pub socket_ids: Vec<String>,

    /// Sockets count
    pub sockets_count: usize,

    /// Exists flag
    pub exists: bool,

    /// Channels set
    pub channels: HashSet<String>,

    /// Members count
    pub members_count: usize,
}

/// Convert UUID string to fixed-size bytes
fn uuid_to_bytes(uuid_str: &str) -> Result<[u8; 16]> {
    uuid::Uuid::parse_str(uuid_str)
        .map(|u| *u.as_bytes())
        .map_err(|e| Error::Other(format!("Failed to parse UUID: {}", e)))
}

/// Convert fixed-size bytes back to UUID string
fn bytes_to_uuid(bytes: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*bytes).to_string()
}

/// Calculate xxh3 hash of a string for fast routing
pub fn calculate_channel_hash(channel: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(channel.as_bytes())
}

impl From<BroadcastMessage> for BinaryBroadcastMessage {
    fn from(msg: BroadcastMessage) -> Self {
        let node_id_bytes = uuid_to_bytes(&msg.node_id).unwrap_or([0u8; 16]);
        let channel_hash = calculate_channel_hash(&msg.channel);

        // The message field contains the JSON string that should be sent to clients
        let raw_client_json = msg.message.into_bytes();

        Self {
            version: BINARY_PROTOCOL_VERSION,
            channel_hash,
            channel: msg.channel,
            node_id_bytes,
            app_id: msg.app_id,
            raw_client_json,
            except_socket_id: msg.except_socket_id,
            timestamp_ms: msg.timestamp_ms,
        }
    }
}

impl From<BinaryBroadcastMessage> for BroadcastMessage {
    fn from(binary: BinaryBroadcastMessage) -> Self {
        Self {
            node_id: bytes_to_uuid(&binary.node_id_bytes),
            app_id: binary.app_id,
            channel: binary.channel,
            message: String::from_utf8_lossy(&binary.raw_client_json).to_string(),
            except_socket_id: binary.except_socket_id,
            timestamp_ms: binary.timestamp_ms,
        }
    }
}

impl TryFrom<RequestBody> for BinaryRequestBody {
    type Error = Error;

    fn try_from(req: RequestBody) -> Result<Self> {
        let request_id_bytes = uuid_to_bytes(&req.request_id)?;
        let node_id_bytes = uuid_to_bytes(&req.node_id)?;
        let channel_hash = req.channel.as_ref().map(|c| calculate_channel_hash(c));

        // Serialize user_info to JSON string first, then to bytes
        let user_info_bytes = req
            .user_info
            .map(|v| serde_json::to_vec(&v))
            .transpose()
            .map_err(|e| Error::Other(format!("Failed to serialize user_info: {}", e)))?;

        Ok(Self {
            version: BINARY_PROTOCOL_VERSION,
            request_id_bytes,
            node_id_bytes,
            app_id: req.app_id,
            request_type: req.request_type,
            channel: req.channel,
            channel_hash,
            socket_id: req.socket_id,
            user_id: req.user_id,
            user_info_bytes,
            timestamp: req.timestamp,
            dead_node_id: req.dead_node_id,
            target_node_id: req.target_node_id,
        })
    }
}

impl TryFrom<BinaryRequestBody> for RequestBody {
    type Error = Error;

    fn try_from(binary: BinaryRequestBody) -> Result<Self> {
        let request_id = bytes_to_uuid(&binary.request_id_bytes);
        let node_id = bytes_to_uuid(&binary.node_id_bytes);

        // Deserialize user_info from JSON bytes if present
        let user_info = binary
            .user_info_bytes
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()
            .map_err(|e| Error::Other(format!("Failed to deserialize user_info: {}", e)))?;

        Ok(Self {
            request_id,
            node_id,
            app_id: binary.app_id,
            request_type: binary.request_type,
            channel: binary.channel,
            socket_id: binary.socket_id,
            user_id: binary.user_id,
            user_info,
            timestamp: binary.timestamp,
            dead_node_id: binary.dead_node_id,
            target_node_id: binary.target_node_id,
        })
    }
}

impl TryFrom<ResponseBody> for BinaryResponseBody {
    type Error = Error;

    fn try_from(resp: ResponseBody) -> Result<Self> {
        let request_id_bytes = uuid_to_bytes(&resp.request_id)?;
        let node_id_bytes = uuid_to_bytes(&resp.node_id)?;

        // Serialize members using JSON (because PresenceMemberInfo contains serde_json::Value)
        let members_bytes = if !resp.members.is_empty() {
            Some(
                serde_json::to_vec(&resp.members)
                    .map_err(|e| Error::Other(format!("Failed to serialize members: {}", e)))?,
            )
        } else {
            None
        };

        // Serialize channels_with_sockets_count using bincode (no JSON values)
        let channels_with_sockets_count_bytes = if !resp.channels_with_sockets_count.is_empty() {
            Some(
                bincode::serialize(&resp.channels_with_sockets_count).map_err(|e| {
                    Error::Other(format!(
                        "Failed to serialize channels_with_sockets_count: {}",
                        e
                    ))
                })?,
            )
        } else {
            None
        };

        Ok(Self {
            version: BINARY_PROTOCOL_VERSION,
            request_id_bytes,
            node_id_bytes,
            app_id: resp.app_id,
            members_bytes,
            channels_with_sockets_count_bytes,
            socket_ids: resp.socket_ids,
            sockets_count: resp.sockets_count,
            exists: resp.exists,
            channels: resp.channels,
            members_count: resp.members_count,
        })
    }
}

impl TryFrom<BinaryResponseBody> for ResponseBody {
    type Error = Error;

    fn try_from(binary: BinaryResponseBody) -> Result<Self> {
        let request_id = bytes_to_uuid(&binary.request_id_bytes);
        let node_id = bytes_to_uuid(&binary.node_id_bytes);

        // Deserialize members from JSON bytes if present
        let members = binary
            .members_bytes
            .map(|bytes| serde_json::from_slice::<HashMap<String, PresenceMemberInfo>>(&bytes))
            .transpose()
            .map_err(|e| Error::Other(format!("Failed to deserialize members: {}", e)))?
            .unwrap_or_default();

        // Deserialize channels_with_sockets_count from bincode bytes if present
        let channels_with_sockets_count = binary
            .channels_with_sockets_count_bytes
            .map(|bytes| bincode::deserialize::<HashMap<String, usize>>(&bytes))
            .transpose()
            .map_err(|e| {
                Error::Other(format!(
                    "Failed to deserialize channels_with_sockets_count: {}",
                    e
                ))
            })?
            .unwrap_or_default();

        Ok(Self {
            request_id,
            node_id,
            app_id: binary.app_id,
            members,
            channels_with_sockets_count,
            socket_ids: binary.socket_ids,
            sockets_count: binary.sockets_count,
            exists: binary.exists,
            channels: binary.channels,
            members_count: binary.members_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_conversion() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let bytes = uuid_to_bytes(uuid_str).unwrap();
        let recovered = bytes_to_uuid(&bytes);
        assert_eq!(uuid_str, recovered);
    }

    #[test]
    fn test_channel_hash() {
        let channel1 = "test-channel";
        let channel2 = "test-channel";
        let channel3 = "different-channel";

        assert_eq!(
            calculate_channel_hash(channel1),
            calculate_channel_hash(channel2)
        );
        assert_ne!(
            calculate_channel_hash(channel1),
            calculate_channel_hash(channel3)
        );
    }

    #[test]
    fn test_broadcast_message_conversion() {
        let original = BroadcastMessage {
            node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            app_id: "test-app".to_string(),
            channel: "test-channel".to_string(),
            message: r#"{"event":"test","data":"payload"}"#.to_string(),
            except_socket_id: Some("socket-123".to_string()),
            timestamp_ms: Some(1234567890.123),
        };

        let binary: BinaryBroadcastMessage = original.clone().into();
        assert_eq!(binary.version, BINARY_PROTOCOL_VERSION);
        assert_eq!(binary.app_id, original.app_id);
        assert_eq!(binary.channel, original.channel);

        let recovered: BroadcastMessage = binary.into();
        assert_eq!(recovered.node_id, original.node_id);
        assert_eq!(recovered.app_id, original.app_id);
        assert_eq!(recovered.channel, original.channel);
        assert_eq!(recovered.message, original.message);
    }

    #[test]
    fn test_bincode_serialization_size() {
        let msg = BroadcastMessage {
            node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            app_id: "test-app".to_string(),
            channel: "test-channel".to_string(),
            message: r#"{"event":"test","data":"payload"}"#.to_string(),
            except_socket_id: None,
            timestamp_ms: Some(1234567890.123),
        };

        let json_size = serde_json::to_vec(&msg).unwrap().len();
        let binary: BinaryBroadcastMessage = msg.into();
        let binary_size = bincode::serialize(&binary).unwrap().len();

        // Binary should be smaller or similar in size
        println!("JSON size: {}, Binary size: {}", json_size, binary_size);
    }
}
