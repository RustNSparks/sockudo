use crate::channel::ChannelType;
use crate::protocol::messages::{MessageData, PusherMessage};
use sonic_rs::{JsonValueTrait, Value};
use std::option::Option;

#[derive(Debug)]
pub struct SubscriptionRequest {
    pub channel: String,
    pub auth: Option<String>,
    pub channel_data: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClientEventRequest {
    pub event: String,
    pub channel: String,
    pub data: Value,
}

#[derive(Debug)]
pub struct SignInRequest {
    pub user_data: String,
    pub auth: String,
}

impl SubscriptionRequest {
    pub fn from_message(message: &PusherMessage) -> crate::error::Result<Self> {
        let (channel, auth, channel_data) = match &message.data {
            Some(MessageData::Structured {
                channel,
                extra,
                channel_data,
                ..
            }) => {
                let ch = channel.as_ref().ok_or_else(|| {
                    crate::error::Error::InvalidMessageFormat("Missing channel field".into())
                })?;
                let channel_data = if ChannelType::from_name(ch) == ChannelType::Presence {
                    Some(channel_data.as_ref().unwrap().clone())
                } else {
                    None
                };
                let auth = extra.get("auth").and_then(Value::as_str).map(String::from);
                (ch.clone(), auth, channel_data)
            }
            Some(MessageData::Json(data)) => {
                let ch = data.get("channel").and_then(Value::as_str).ok_or_else(|| {
                    crate::error::Error::InvalidMessageFormat("Missing channel field".into())
                })?;
                let auth = data.get("auth").and_then(Value::as_str).map(String::from);
                let channel_data = data
                    .get("channel_data")
                    .and_then(Value::as_str)
                    .map(String::from);
                (ch.to_string(), auth, channel_data)
            }
            Some(MessageData::String(s)) => {
                let data: Value = sonic_rs::from_str(s).map_err(|_| {
                    crate::error::Error::InvalidMessageFormat(
                        "Failed to parse subscription data".into(),
                    )
                })?;
                let ch = data.get("channel").and_then(Value::as_str).ok_or_else(|| {
                    crate::error::Error::InvalidMessageFormat(
                        "Missing channel field in string data".into(),
                    )
                })?;
                let auth = data.get("auth").and_then(Value::as_str).map(String::from);
                let channel_data = data
                    .get("channel_data")
                    .and_then(Value::as_str)
                    .map(String::from);
                (ch.to_string(), auth, channel_data)
            }
            _ => {
                return Err(crate::error::Error::InvalidMessageFormat(
                    "Invalid subscription data format".into(),
                ));
            }
        };

        Ok(Self {
            channel,
            auth,
            channel_data,
        })
    }
}

impl SignInRequest {
    pub fn from_message(message: &PusherMessage) -> crate::error::Result<Self> {
        let extract_field = |data: &Value, field: &str| -> crate::error::Result<String> {
            data.get(field)
                .and_then(Value::as_str)
                .map(String::from)
                .ok_or_else(|| {
                    crate::error::Error::Auth(format!("Missing '{field}' field in signin data"))
                })
        };

        match &message.data {
            Some(MessageData::Json(data)) => Ok(Self {
                user_data: extract_field(data, "user_data")?,
                auth: extract_field(data, "auth")?,
            }),
            Some(MessageData::Structured {
                user_data, extra, ..
            }) => Ok(Self {
                user_data: user_data.as_ref().cloned().ok_or_else(|| {
                    crate::error::Error::Auth("Missing 'user_data' field in signin data".into())
                })?,
                auth: extra
                    .get("auth")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .ok_or_else(|| {
                        crate::error::Error::Auth("Missing 'auth' field in signin data".into())
                    })?,
            }),
            _ => Err(crate::error::Error::InvalidMessageFormat(
                "Invalid signin data format".into(),
            )),
        }
    }
}
