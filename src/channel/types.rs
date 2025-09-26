use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelType {
    Public,
    Private,
    Presence,
    PrivateEncrypted,
    Cache,
}

impl ChannelType {
    pub fn from_name(channel_name: &str) -> Self {
        // Check cache channels first using the utility function
        if crate::utils::is_cache_channel(channel_name) {
            return Self::Cache;
        }

        match channel_name.split_once('-') {
            Some(("private", "encrypted")) => Self::PrivateEncrypted,
            Some(("private", _)) => Self::Private,
            Some(("presence", _)) => Self::Presence,
            _ => Self::Public,
        }
    }

    pub fn requires_authentication(&self) -> bool {
        matches!(
            self,
            ChannelType::Private | ChannelType::Presence | ChannelType::PrivateEncrypted
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::Public => "public",
            ChannelType::Private => "private",
            ChannelType::Presence => "presence",
            ChannelType::PrivateEncrypted => "private_encrypted",
            ChannelType::Cache => "cache",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceMemberInfo {
    pub user_id: String,
    pub user_info: Option<serde_json::Value>,
}
