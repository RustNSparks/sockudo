use crate::webhook::types::Webhook;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct App {
    pub id: String,
    pub key: String,
    pub secret: String,
    #[serde(deserialize_with = "deserialize_flexible_number")]
    pub max_connections: u32,
    pub enable_client_messages: bool,
    pub enabled: bool,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_backend_events_per_second: Option<u32>,
    #[serde(deserialize_with = "deserialize_flexible_number")]
    pub max_client_events_per_second: u32,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_read_requests_per_second: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_presence_members_per_channel: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_presence_member_size_in_kb: Option<u32>,
    #[serde(default)]
    pub max_channel_name_length: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_event_channels_at_once: Option<u32>,
    #[serde(default)]
    pub max_event_name_length: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_event_payload_in_kb: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_number_from_string")]
    pub max_event_batch_size: Option<u32>,
    #[serde(default)]
    pub enable_user_authentication: Option<bool>,
    #[serde(default)]
    pub webhooks: Option<Vec<Webhook>>,
    #[serde(default)]
    pub enable_watchlist_events: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_and_validate_origins")]
    pub allowed_origins: Option<Vec<String>>,
}

// Helper functions to deserialize numbers from strings or numbers
fn deserialize_flexible_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Visitor};
    use std::fmt;

    struct FlexibleU32Visitor;

    impl<'de> Visitor<'de> for FlexibleU32Visitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a number or string containing a number")
        }

        fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(v)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            u32::try_from(v).map_err(|_| {
                if v < 0 {
                    E::custom(format!("number {} cannot be negative", v))
                } else {
                    E::custom(format!("number {} is too large for u32", v))
                }
            })
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            u32::try_from(v).map_err(|_| E::custom(format!("number {} is too large for u32", v)))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            v.parse::<u32>().map_err(E::custom)
        }
    }

    deserializer.deserialize_any(FlexibleU32Visitor)
}

fn deserialize_optional_number_from_string<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        Some(s) => Ok(Some(s.parse::<u32>().map_err(D::Error::custom)?)),
        None => Ok(None),
    }
}

fn deserialize_and_validate_origins<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = Option::<Vec<String>>::deserialize(deserializer)?;

    if let Some(ref origins) = value {
        // Validate origin patterns at configuration load time
        if let Err(validation_error) =
            crate::adapter::handler::origin_validation::OriginValidator::validate_patterns(origins)
        {
            return Err(D::Error::custom(format!(
                "Origin pattern validation failed: {}",
                validation_error
            )));
        }
    }

    Ok(value)
}

// Implementation with default values
