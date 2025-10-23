use serde::{
    Deserialize, Serialize,
    de::{
        self, Deserializer, MapAccess, SeqAccess, Visitor,
        value::{EnumAccessDeserializer, SeqAccessDeserializer},
    },
};
use sonic_rs::{Value, json};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceData {
    pub ids: Vec<String>,
    pub hash: HashMap<String, Option<Value>>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MessageData {
    String(String),
    Structured {
        #[serde(skip_serializing_if = "Option::is_none")]
        channel_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_data: Option<String>,
        extra: HashMap<String, Value>,
    },
    Json(Value),
}

impl MessageData {
    fn json_from_serializable<T, E>(value: T) -> Result<Self, E>
    where
        T: Serialize,
        E: de::Error,
    {
        sonic_rs::to_value(&value)
            .map(MessageData::Json)
            .map_err(|err| E::custom(err.to_string()))
    }
}

impl<'de> Deserialize<'de> for MessageData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MessageDataVisitor;

        impl<'de> Visitor<'de> for MessageDataVisitor {
            type Value = MessageData;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("string, object, or JSON value for MessageData")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(MessageData::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(MessageData::String(value))
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(MessageData::String(value.to_owned()))
            }

            fn visit_char<E>(self, value: char) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(value)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                MessageData::json_from_serializable::<_, E>(())
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_any(MessageDataVisitor)
            }

            fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_any(MessageDataVisitor)
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let value = Value::deserialize(SeqAccessDeserializer::new(seq))?;
                Ok(MessageData::Json(value))
            }

            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where
                A: de::EnumAccess<'de>,
            {
                let value = Value::deserialize(EnumAccessDeserializer::new(data))?;
                Ok(MessageData::Json(value))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut channel_data: Option<Option<String>> = None;
                let mut channel: Option<Option<String>> = None;
                let mut user_data: Option<Option<String>> = None;
                let mut extra = HashMap::with_capacity(map.size_hint().unwrap_or(0));

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "channel_data" => {
                            if channel_data.is_some() {
                                return Err(de::Error::duplicate_field("channel_data"));
                            }
                            channel_data = Some(map.next_value()?);
                        }
                        "channel" => {
                            if channel.is_some() {
                                return Err(de::Error::duplicate_field("channel"));
                            }
                            channel = Some(map.next_value()?);
                        }
                        "user_data" => {
                            if user_data.is_some() {
                                return Err(de::Error::duplicate_field("user_data"));
                            }
                            user_data = Some(map.next_value()?);
                        }
                        _ => {
                            let value = map.next_value()?;
                            extra.insert(key, value);
                        }
                    }
                }

                Ok(MessageData::Structured {
                    channel_data: channel_data.unwrap_or(None),
                    channel: channel.unwrap_or(None),
                    user_data: user_data.unwrap_or(None),
                    extra,
                })
            }
        }

        deserializer.deserialize_any(MessageDataVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorData {
    pub code: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PusherMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<MessageData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PusherApiMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ApiMessageData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPusherApiMessage {
    pub batch: Vec<PusherApiMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiMessageData {
    String(String),
    Json(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentPusherMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<MessageData>,
}

// Helper implementations
impl MessageData {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            MessageData::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn into_string(self) -> Option<String> {
        match self {
            MessageData::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_value(&self) -> Option<&Value> {
        match self {
            MessageData::Structured { extra, .. } => extra.values().next(),
            _ => None,
        }
    }
}

impl From<String> for MessageData {
    fn from(s: String) -> Self {
        MessageData::String(s)
    }
}

impl From<Value> for MessageData {
    fn from(v: Value) -> Self {
        MessageData::Json(v)
    }
}

impl PusherMessage {
    pub fn connection_established(socket_id: String, activity_timeout: u64) -> Self {
        Self {
            event: Some("pusher:connection_established".to_string()),
            data: Some(MessageData::from(
                json!({
                    "socket_id": socket_id,
                    "activity_timeout": activity_timeout  // Now configurable
                })
                .to_string(),
            )),
            channel: None,
            name: None,
            user_id: None,
        }
    }
    pub fn subscription_succeeded(channel: String, presence_data: Option<PresenceData>) -> Self {
        let data_obj = if let Some(data) = presence_data {
            json!({
                "presence": {
                    "ids": data.ids,
                    "hash": data.hash,
                    "count": data.count
                }
            })
        } else {
            json!({})
        };

        Self {
            event: Some("pusher_internal:subscription_succeeded".to_string()),
            channel: Some(channel),
            data: Some(MessageData::String(data_obj.to_string())),
            name: None,
            user_id: None,
        }
    }

    pub fn error(code: u16, message: String, channel: Option<String>) -> Self {
        Self {
            event: Some("pusher:error".to_string()),
            data: Some(MessageData::Json(json!({
                "code": code,
                "message": message
            }))),
            channel,
            name: None,
            user_id: None,
        }
    }

    pub fn ping() -> Self {
        Self {
            event: Some("pusher:ping".to_string()),
            data: None,
            channel: None,
            name: None,
            user_id: None,
        }
    }
    pub fn channel_event<S: Into<String>>(event: S, channel: S, data: Value) -> Self {
        Self {
            event: Some(event.into()),
            channel: Some(channel.into()),
            data: Some(MessageData::String(data.to_string())),
            name: None,
            user_id: None,
        }
    }

    pub fn member_added(channel: String, user_id: String, user_info: Option<Value>) -> Self {
        Self {
            event: Some("pusher_internal:member_added".to_string()),
            channel: Some(channel),
            // FIX: Use MessageData::String with JSON-encoded string instead of MessageData::Json
            data: Some(MessageData::String(
                json!({
                    "user_id": user_id,
                    "user_info": user_info.unwrap_or_else(|| json!({}))
                })
                .to_string(),
            )),
            name: None,
            user_id: None,
        }
    }

    pub fn member_removed(channel: String, user_id: String) -> Self {
        Self {
            event: Some("pusher_internal:member_removed".to_string()),
            channel: Some(channel),
            // FIX: Also apply same fix to member_removed for consistency
            data: Some(MessageData::String(
                json!({
                    "user_id": user_id
                })
                .to_string(),
            )),
            name: None,
            user_id: None,
        }
    }

    // New helper method for pong response
    pub fn pong() -> Self {
        Self {
            event: Some("pusher:pong".to_string()),
            data: None,
            channel: None,
            name: None,
            user_id: None,
        }
    }

    // Helper for creating channel info response
    pub fn channel_info(
        occupied: bool,
        subscription_count: Option<u64>,
        user_count: Option<u64>,
        cache_data: Option<(String, Duration)>,
    ) -> Value {
        let mut response = json!({
            "occupied": occupied
        });

        if let Some(count) = subscription_count {
            response["subscription_count"] = json!(count);
        }

        if let Some(count) = user_count {
            response["user_count"] = json!(count);
        }

        if let Some((data, ttl)) = cache_data {
            response["cache"] = json!({
                "data": data,
                "ttl": ttl.as_secs()
            });
        }

        response
    }

    // Helper for creating channels list response
    pub fn channels_list(channels_info: HashMap<String, Value>) -> Value {
        json!({
            "channels": channels_info
        })
    }

    // Helper for creating user list response
    pub fn user_list(user_ids: Vec<String>) -> Value {
        let users = user_ids
            .into_iter()
            .map(|id| json!({ "id": id }))
            .collect::<Vec<_>>();

        json!({ "users": users })
    }

    // Helper for batch events response
    pub fn batch_response(batch_info: Vec<Value>) -> Value {
        json!({ "batch": batch_info })
    }

    // Helper for simple success response
    pub fn success_response() -> Value {
        json!({ "ok": true })
    }

    pub fn watchlist_online_event(user_ids: Vec<String>) -> Self {
        Self {
            event: Some("online".to_string()),
            channel: None, // Watchlist events don't use channels
            name: None,
            data: Some(MessageData::Json(json!({
                "user_ids": user_ids
            }))),
            user_id: None,
        }
    }

    pub fn watchlist_offline_event(user_ids: Vec<String>) -> Self {
        Self {
            event: Some("offline".to_string()),
            channel: None,
            name: None,
            data: Some(MessageData::Json(json!({
                "user_ids": user_ids
            }))),
            user_id: None,
        }
    }

    pub fn cache_miss_event(channel: String) -> Self {
        Self {
            event: Some("pusher:cache_miss".to_string()),
            channel: Some(channel),
            data: Some(MessageData::String("{}".to_string())),
            name: None,
            user_id: None,
        }
    }

    pub fn signin_success(user_data: String) -> Self {
        Self {
            event: Some("pusher:signin_success".to_string()),
            data: Some(MessageData::Json(json!({
                "user_data": user_data
            }))),
            channel: None,
            name: None,
            user_id: None,
        }
    }
}

// Add a helper extension trait for working with info parameters
pub trait InfoQueryParser {
    fn parse_info(&self) -> Vec<&str>;
    fn wants_user_count(&self) -> bool;
    fn wants_subscription_count(&self) -> bool;
    fn wants_cache(&self) -> bool;
}

impl InfoQueryParser for Option<&String> {
    fn parse_info(&self) -> Vec<&str> {
        self.map(|s| s.split(',').collect::<Vec<_>>())
            .unwrap_or_default()
    }

    fn wants_user_count(&self) -> bool {
        self.parse_info().contains(&"user_count")
    }

    fn wants_subscription_count(&self) -> bool {
        self.parse_info().contains(&"subscription_count")
    }

    fn wants_cache(&self) -> bool {
        self.parse_info().contains(&"cache")
    }
}
