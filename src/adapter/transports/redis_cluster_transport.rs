use crate::adapter::horizontal_adapter::{BroadcastMessage, RequestBody, ResponseBody};
use crate::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use crate::error::{Error, Result};
use crate::options::RedisClusterAdapterConfig;
use async_trait::async_trait;
use redis::AsyncCommands;
use redis::cluster::{ClusterClient, ClusterClientBuilder};
use tracing::{debug, error};

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
    connection: redis::cluster_async::ClusterConnection,
    broadcast_channel: String,
    request_channel: String,
    response_channel: String,
    config: RedisClusterAdapterConfig,
}

#[async_trait]
impl HorizontalTransport for RedisClusterTransport {
    type Config = RedisClusterAdapterConfig;

    async fn new(config: Self::Config) -> Result<Self> {
        let client = ClusterClient::new(config.nodes.clone())
            .map_err(|e| Error::Redis(format!("Failed to create Redis Cluster client: {e}")))?;

        let connection = client
            .get_async_connection()
            .await
            .map_err(|e| Error::Redis(format!("Failed to connect to Redis Cluster: {e}")))?;

        let broadcast_channel = format!("{}:#broadcast", config.prefix);
        let request_channel = format!("{}:#requests", config.prefix);
        let response_channel = format!("{}:#responses", config.prefix);

        Ok(Self {
            client,
            connection,
            broadcast_channel,
            request_channel,
            response_channel,
            config,
        })
    }

    async fn publish_broadcast(&self, message: &BroadcastMessage) -> Result<()> {
        let broadcast_json = serde_json::to_string(message)?;
        let mut conn = self.connection.clone();
        conn.publish::<_, _, ()>(&self.broadcast_channel, broadcast_json)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish broadcast: {e}")))?;

        Ok(())
    }

    async fn publish_request(&self, request: &RequestBody) -> Result<()> {
        let request_json = serde_json::to_string(request)
            .map_err(|e| Error::Other(format!("Failed to serialize request: {e}")))?;

        let mut conn = self.connection.clone();
        let subscriber_count: i32 = conn
            .publish(&self.request_channel, &request_json)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish request: {e}")))?;

        debug!(
            "Broadcasted request {} to {} subscribers",
            request.request_id, subscriber_count
        );

        Ok(())
    }

    async fn publish_response(&self, response: &ResponseBody) -> Result<()> {
        let response_json = serde_json::to_string(response)
            .map_err(|e| Error::Other(format!("Failed to serialize response: {e}")))?;

        let mut conn = self.connection.clone();
        conn.publish::<_, _, ()>(&self.response_channel, response_json)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish response: {e}")))?;

        Ok(())
    }

    async fn start_listeners(&self, handlers: TransportHandlers) -> Result<()> {
        // Clone needed values for the async task
        let pub_connection = self.connection.clone();
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

                let channel = match &push_info.data[0] {
                    redis::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
                        Ok(s) => s,
                        Err(_) => {
                            error!("Failed to parse channel name from bulk string bytes");
                            continue;
                        }
                    },
                    redis::Value::SimpleString(s) => s.clone(),
                    redis::Value::VerbatimString { format: _, text } => text.clone(),
                    _ => {
                        error!("Unexpected channel format: {:?}", push_info.data[0]);
                        continue;
                    }
                };

                let payload = match &push_info.data[1] {
                    redis::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
                        Ok(s) => s,
                        Err(_) => {
                            error!("Failed to parse payload from bulk string bytes");
                            continue;
                        }
                    },
                    redis::Value::SimpleString(s) => s.clone(),
                    redis::Value::VerbatimString { format: _, text } => text.clone(),
                    _ => {
                        error!("Unexpected payload format: {:?}", push_info.data[1]);
                        continue;
                    }
                };

                // Process the message in a separate task
                let broadcast_handler = handlers.on_broadcast.clone();
                let request_handler = handlers.on_request.clone();
                let response_handler = handlers.on_response.clone();
                let pub_connection_clone = pub_connection.clone();
                let broadcast_channel_clone = broadcast_channel.clone();
                let request_channel_clone = request_channel.clone();
                let response_channel_clone = response_channel.clone();

                tokio::spawn(async move {
                    if channel == broadcast_channel_clone {
                        // Handle broadcast message
                        if let Ok(broadcast) = serde_json::from_str::<BroadcastMessage>(&payload) {
                            broadcast_handler(broadcast).await;
                        }
                    } else if channel == request_channel_clone {
                        // Handle request message
                        if let Ok(request) = serde_json::from_str::<RequestBody>(&payload) {
                            let response_result = request_handler(request).await;

                            if let Ok(response) = response_result
                                && let Ok(response_json) = serde_json::to_string(&response)
                            {
                                let mut conn = pub_connection_clone.clone();
                                let _ = conn
                                    .publish::<_, _, ()>(&response_channel_clone, response_json)
                                    .await;
                            }
                        }
                    } else if channel == response_channel_clone {
                        // Handle response message
                        if let Ok(response) = serde_json::from_str::<ResponseBody>(&payload) {
                            response_handler(response).await;
                        }
                    }
                });
            }
        });

        Ok(())
    }

    async fn get_node_count(&self) -> Result<usize> {
        let mut conn = self.connection.clone();
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
        // Use a dedicated connection for health check to avoid impacting main operations
        let mut conn =
            self.client.get_async_connection().await.map_err(|e| {
                Error::Redis(format!("Failed to acquire health check connection: {e}"))
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
