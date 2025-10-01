use crate::adapter::binary_protocol::{
    BinaryBroadcastMessage, BinaryRequestBody, BinaryResponseBody, bincode_config,
};
use crate::adapter::horizontal_adapter::{BroadcastMessage, RequestBody, ResponseBody};
use crate::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use crate::error::{Error, Result};
use crate::options::RedisClusterAdapterConfig;
use async_trait::async_trait;
use redis::AsyncCommands;
use redis::cluster::{ClusterClient, ClusterClientBuilder};
use tracing::{debug, error, warn};

/// Helper function to convert redis::Value to String
fn value_to_string(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        redis::Value::SimpleString(s) => Some(s.clone()),
        redis::Value::VerbatimString { format: _, text } => Some(text.clone()),
        _ => None,
    }
}

/// Helper function to convert redis::Value to bytes (for binary data)
fn value_to_bytes(v: &redis::Value) -> Option<Vec<u8>> {
    match v {
        redis::Value::BulkString(bytes) => Some(bytes.clone()),
        _ => None,
    }
}

impl TransportConfig for RedisClusterAdapterConfig {
    fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Redis Cluster transport implementation
#[derive(Clone)]
pub struct RedisClusterTransport {
    client: ClusterClient,
    broadcast_channel: String,
    request_channel: String,
    response_channel: String,
    config: RedisClusterAdapterConfig,
}

#[async_trait]
impl HorizontalTransport for RedisClusterTransport {
    type Config = RedisClusterAdapterConfig;

    async fn new(config: Self::Config) -> Result<Self> {
        let client = ClusterClientBuilder::new(config.nodes.clone())
            .retries(3)
            .read_from_replicas()
            .build()
            .map_err(|e| Error::Redis(format!("Failed to create Redis Cluster client: {e}")))?;

        let broadcast_channel = format!("{}:#broadcast", config.prefix);
        let request_channel = format!("{}:#requests", config.prefix);
        let response_channel = format!("{}:#responses", config.prefix);

        Ok(Self {
            client,
            broadcast_channel,
            request_channel,
            response_channel,
            config,
        })
    }

    async fn publish_broadcast(&self, message: &BroadcastMessage) -> Result<()> {
        // Convert to binary format
        let binary_msg: BinaryBroadcastMessage = message.clone().into();
        let broadcast_bytes = bincode::encode_to_vec(&binary_msg, bincode_config())
            .map_err(|e| Error::Other(format!("Failed to serialize broadcast: {}", e)))?;

        // Use client's internal connection pooling - this is efficient
        let mut conn = self.client.get_async_connection().await.map_err(|e| {
            Error::Redis(format!(
                "Failed to get cluster connection for broadcast: {e}"
            ))
        })?;

        conn.publish::<_, _, ()>(&self.broadcast_channel, broadcast_bytes.as_slice())
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish broadcast: {e}")))?;

        Ok(())
    }

    async fn publish_request(&self, request: &RequestBody) -> Result<()> {
        // Convert to binary format
        let binary_req: BinaryRequestBody = request.clone().try_into()?;
        let request_bytes = bincode::encode_to_vec(&binary_req, bincode_config())
            .map_err(|e| Error::Other(format!("Failed to serialize request: {}", e)))?;

        // Use client's internal connection pooling - this is efficient for cluster
        let mut conn = self.client.get_async_connection().await.map_err(|e| {
            Error::Redis(format!("Failed to get cluster connection for request: {e}"))
        })?;

        let subscriber_count: i32 = conn
            .publish(&self.request_channel, request_bytes.as_slice())
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish request: {e}")))?;

        debug!(
            "Broadcasted request {} to {} subscribers",
            request.request_id, subscriber_count
        );

        Ok(())
    }

    async fn publish_response(&self, response: &ResponseBody) -> Result<()> {
        // Convert to binary format
        let binary_resp: BinaryResponseBody = response.clone().try_into()?;
        let response_bytes = bincode::encode_to_vec(&binary_resp, bincode_config())
            .map_err(|e| Error::Other(format!("Failed to serialize response: {}", e)))?;

        // Use client's internal connection pooling - this is efficient for cluster
        let mut conn = self.client.get_async_connection().await.map_err(|e| {
            Error::Redis(format!(
                "Failed to get cluster connection for response: {e}"
            ))
        })?;

        conn.publish::<_, _, ()>(&self.response_channel, response_bytes.as_slice())
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish response: {e}")))?;

        Ok(())
    }

    async fn start_listeners(&self, handlers: TransportHandlers) -> Result<()> {
        // Clone needed values for the async task
        let client = self.client.clone();
        let broadcast_channel = self.broadcast_channel.clone();
        let request_channel = self.request_channel.clone();
        let response_channel = self.response_channel.clone();
        let nodes = self.config.nodes.clone();

        // Create a separate channel for receiving PubSub messages
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Create a new client with RESP3 protocol for PubSub
        let sub_client = ClusterClientBuilder::new(nodes)
            .use_protocol(redis::ProtocolVersion::RESP3)
            .push_sender(tx)
            .build()
            .map_err(|e| Error::Redis(format!("Failed to create PubSub client: {e}")))?;

        // Spawn the main listener task
        tokio::spawn(async move {
            // Create a connection for PubSub
            let mut pubsub = match sub_client.get_async_connection().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Failed to get pubsub connection: {}", e);
                    return;
                }
            };

            // Subscribe to all channels
            if let Err(e) = pubsub
                .subscribe(&[&broadcast_channel, &request_channel, &response_channel])
                .await
            {
                error!("Failed to subscribe to channels: {}", e);
                return;
            }

            debug!(
                "Redis Cluster transport listening on channels: {}, {}, {}",
                broadcast_channel, request_channel, response_channel
            );

            // Process messages from the channel - PushInfo is the message type for RESP3
            while let Some(push_info) = rx.recv().await {
                // Extract channel and payload from PushInfo
                if push_info.kind != redis::PushKind::Message {
                    continue; // Skip non-message push notifications
                }

                // PushInfo.data for messages should be [channel, payload]
                if push_info.data.len() < 2 {
                    error!("Invalid push message format: {:?}", push_info);
                    continue;
                }

                let channel = match value_to_string(&push_info.data[0]) {
                    Some(s) => s,
                    None => {
                        error!("Failed to parse channel name: {:?}", push_info.data[0]);
                        continue;
                    }
                };

                let payload_bytes = match value_to_bytes(&push_info.data[1]) {
                    Some(bytes) => bytes,
                    None => {
                        error!("Failed to parse payload: {:?}", push_info.data[1]);
                        continue;
                    }
                };

                // Process the message in a separate task
                let broadcast_handler = handlers.on_broadcast.clone();
                let request_handler = handlers.on_request.clone();
                let response_handler = handlers.on_response.clone();
                let client_clone = client.clone();
                let broadcast_channel_clone = broadcast_channel.clone();
                let request_channel_clone = request_channel.clone();
                let response_channel_clone = response_channel.clone();

                tokio::spawn(async move {
                    if channel == broadcast_channel_clone {
                        // Handle broadcast message - deserialize from binary
                        if let Ok((binary_msg, _)) =
                            bincode::decode_from_slice::<BinaryBroadcastMessage, _>(
                                &payload_bytes,
                                bincode_config(),
                            )
                        {
                            let broadcast: BroadcastMessage = binary_msg.into();
                            broadcast_handler(broadcast).await;
                        }
                    } else if channel == request_channel_clone {
                        // Handle request message - deserialize from binary
                        if let Ok((binary_req, _)) =
                            bincode::decode_from_slice::<BinaryRequestBody, _>(
                                &payload_bytes,
                                bincode_config(),
                            )
                            && let Ok(request) = RequestBody::try_from(binary_req)
                        {
                            let response_result = request_handler(request).await;

                            if let Ok(response) = response_result {
                                // Serialize response to binary
                                if let Ok(binary_resp) = BinaryResponseBody::try_from(response)
                                    && let Ok(response_bytes) =
                                        bincode::encode_to_vec(&binary_resp, bincode_config())
                                {
                                    // Use client's connection pooling for response publishing
                                    if let Ok(mut conn) = client_clone.get_async_connection().await
                                    {
                                        let _ = conn
                                            .publish::<_, _, ()>(
                                                &response_channel_clone,
                                                response_bytes.as_slice(),
                                            )
                                            .await;
                                    }
                                }
                            }
                        }
                    } else if channel == response_channel_clone {
                        // Handle response message - deserialize from binary
                        if let Ok((binary_resp, _)) =
                            bincode::decode_from_slice::<BinaryResponseBody, _>(
                                &payload_bytes,
                                bincode_config(),
                            )
                        {
                            if let Ok(response) = ResponseBody::try_from(binary_resp) {
                                response_handler(response).await;
                            }
                        } else {
                            warn!("Failed to parse binary response message");
                        }
                    }
                });
            }
        });

        Ok(())
    }

    async fn get_node_count(&self) -> Result<usize> {
        // Use client's connection pooling for node count queries
        let mut conn = self.client.get_async_connection().await.map_err(|e| {
            Error::Redis(format!(
                "Failed to get cluster connection for node count: {e}"
            ))
        })?;

        let result: redis::RedisResult<Vec<redis::Value>> = redis::cmd("PUBSUB")
            .arg("NUMSUB")
            .arg(&self.request_channel)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(values) => {
                if values.len() >= 2 {
                    if let redis::Value::Int(count) = values[1] {
                        Ok((count as usize).max(1))
                    } else {
                        Ok(1)
                    }
                } else {
                    Ok(1)
                }
            }
            Err(e) => {
                error!("Failed to execute PUBSUB NUMSUB: {}", e);
                Ok(1)
            }
        }
    }

    async fn check_health(&self) -> Result<()> {
        // Use client's connection pooling for health checks
        let mut conn = self.client.get_async_connection().await.map_err(|e| {
            Error::Redis(format!(
                "Failed to get cluster connection for health check: {e}"
            ))
        })?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| Error::Redis(format!("Cluster health check PING failed: {e}")))?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(Error::Redis(format!(
                "Cluster PING returned unexpected response: {response}"
            )))
        }
    }
}
