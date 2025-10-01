use crate::adapter::binary_protocol::{
    BinaryBroadcastMessage, BinaryRequestBody, BinaryResponseBody,
};
use crate::adapter::horizontal_adapter::{BroadcastMessage, RequestBody, ResponseBody};
use crate::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use crate::error::{Error, Result};
use async_trait::async_trait;
use futures::StreamExt;
use redis::AsyncCommands;
use tracing::{debug, error, warn};

/// Redis adapter configuration
#[derive(Debug, Clone)]
pub struct RedisAdapterConfig {
    pub url: String,
    pub prefix: String,
    pub request_timeout_ms: u64,
    pub cluster_mode: bool,
}

impl Default for RedisAdapterConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379/".to_string(),
            prefix: "sockudo".to_string(),
            request_timeout_ms: 5000,
            cluster_mode: false,
        }
    }
}

impl TransportConfig for RedisAdapterConfig {
    fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Redis transport implementation
#[derive(Clone)]
pub struct RedisTransport {
    client: redis::Client,
    connection: redis::aio::ConnectionManager,
    events_connection: redis::aio::ConnectionManager,
    broadcast_channel: String,
    request_channel: String,
    response_channel: String,
}

#[async_trait]
impl HorizontalTransport for RedisTransport {
    type Config = RedisAdapterConfig;

    async fn new(config: Self::Config) -> Result<Self> {
        let client = redis::Client::open(&*config.url)
            .map_err(|e| Error::Redis(format!("Failed to create Redis client: {e}")))?;

        // Use ConnectionManager consistently for auto-reconnection
        let connection_manager_config = redis::aio::ConnectionManagerConfig::new()
            .set_number_of_retries(5)
            .set_exponent_base(2)
            .set_factor(500)
            .set_max_delay(5000);

        let connection = client
            .get_connection_manager_with_config(connection_manager_config.clone())
            .await
            .map_err(|e| Error::Redis(format!("Failed to connect to Redis: {e}")))?;

        // Create ConnectionManager for events API broadcasts (reuse same config)
        let events_connection = client
            .get_connection_manager_with_config(connection_manager_config)
            .await
            .map_err(|e| {
                Error::Redis(format!(
                    "Failed to create Redis connection manager for events: {e}"
                ))
            })?;

        let broadcast_channel = format!("{}:#broadcast", config.prefix);
        let request_channel = format!("{}:#requests", config.prefix);
        let response_channel = format!("{}:#responses", config.prefix);

        Ok(Self {
            client,
            connection,
            events_connection,
            broadcast_channel,
            request_channel,
            response_channel,
        })
    }

    async fn publish_broadcast(&self, message: &BroadcastMessage) -> Result<()> {
        // Convert to binary format
        let binary_msg: BinaryBroadcastMessage = message.clone().into();
        let broadcast_bytes = bincode::encode_to_vec(&binary_msg, bincode::config::standard())
            .map_err(|e| Error::Other(format!("Failed to serialize broadcast: {}", e)))?;

        // Retry broadcast with exponential backoff to handle connection recovery
        let mut retry_delay = 100u64; // Start with 100ms
        const MAX_RETRIES: u32 = 3;
        const MAX_RETRY_DELAY: u64 = 1000; // Max 1 second

        for attempt in 0..=MAX_RETRIES {
            let mut conn = self.events_connection.clone();
            match conn
                .publish::<_, _, i32>(&self.broadcast_channel, &broadcast_bytes)
                .await
            {
                Ok(_subscriber_count) => {
                    if attempt > 0 {
                        debug!("Broadcast succeeded on retry attempt {}", attempt);
                    }
                    return Ok(());
                }
                Err(e) => {
                    if attempt == MAX_RETRIES {
                        return Err(Error::Redis(format!(
                            "Failed to publish broadcast after {} attempts: {}",
                            MAX_RETRIES + 1,
                            e
                        )));
                    }

                    warn!(
                        "Broadcast attempt {} failed: {}, retrying in {}ms",
                        attempt + 1,
                        e,
                        retry_delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                }
            }
        }

        // This should never be reached due to the loop logic, but return error for safety
        Err(Error::Redis(
            "All retry attempts failed unexpectedly".to_string(),
        ))
    }

    async fn publish_request(&self, request: &RequestBody) -> Result<()> {
        // Convert to binary format
        let binary_req: BinaryRequestBody = request.clone().try_into()?;
        let request_bytes = bincode::encode_to_vec(&binary_req, bincode::config::standard())
            .map_err(|e| Error::Other(format!("Failed to serialize request: {}", e)))?;

        let mut conn = self.connection.clone();
        let subscriber_count: i32 = conn
            .publish(&self.request_channel, &request_bytes)
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
        let response_bytes = bincode::encode_to_vec(&binary_resp, bincode::config::standard())
            .map_err(|e| Error::Other(format!("Failed to serialize response: {}", e)))?;

        let mut conn = self.connection.clone();
        let _: () = conn
            .publish(&self.response_channel, response_bytes)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish response: {e}")))?;

        Ok(())
    }

    async fn start_listeners(&self, handlers: TransportHandlers) -> Result<()> {
        let sub_client = self.client.clone();
        let pub_connection = self.connection.clone();
        let broadcast_channel = self.broadcast_channel.clone();
        let request_channel = self.request_channel.clone();
        let response_channel = self.response_channel.clone();

        tokio::spawn(async move {
            let mut retry_delay = 500u64; // Start with 500ms delay
            const MAX_RETRY_DELAY: u64 = 10_000; // Max 10 seconds

            loop {
                debug!("Attempting to establish pub/sub connection...");

                let mut pubsub = match sub_client.get_async_pubsub().await {
                    Ok(pubsub) => {
                        retry_delay = 500; // Reset retry delay on success
                        debug!("Pub/sub connection established successfully");
                        pubsub
                    }
                    Err(e) => {
                        error!(
                            "Failed to get pubsub connection: {}, retrying in {}ms",
                            e, retry_delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;
                        retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                        continue;
                    }
                };

                if let Err(e) = pubsub
                    .subscribe(&[&broadcast_channel, &request_channel, &response_channel])
                    .await
                {
                    error!(
                        "Failed to subscribe to channels: {}, retrying in {}ms",
                        e, retry_delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                    continue;
                }

                debug!(
                    "Redis transport listening on channels: {}, {}, {}",
                    broadcast_channel, request_channel, response_channel
                );

                let mut message_stream = pubsub.on_message();
                let mut connection_broken = false;

                while let Some(msg) = message_stream.next().await {
                    let channel: String = msg.get_channel_name().to_string();
                    let payload_result: redis::RedisResult<String> = msg.get_payload();

                    if let Ok(payload) = payload_result {
                        let broadcast_handler = handlers.on_broadcast.clone();
                        let request_handler = handlers.on_request.clone();
                        let response_handler = handlers.on_response.clone();
                        let pub_connection_clone = pub_connection.clone();
                        let broadcast_channel_clone = broadcast_channel.clone();
                        let request_channel_clone = request_channel.clone();
                        let response_channel_clone = response_channel.clone();

                        tokio::spawn(async move {
                            if channel == broadcast_channel_clone {
                                // Handle broadcast message - deserialize from binary
                                if let Ok((binary_msg, _)) =
                                    bincode::decode_from_slice::<BinaryBroadcastMessage, _>(
                                        payload.as_bytes(),
                                        bincode::config::standard(),
                                    )
                                {
                                    let broadcast: BroadcastMessage = binary_msg.into();
                                    broadcast_handler(broadcast).await;
                                }
                            } else if channel == request_channel_clone {
                                // Handle request message - deserialize from binary
                                if let Ok((binary_req, _)) =
                                    bincode::decode_from_slice::<BinaryRequestBody, _>(
                                        payload.as_bytes(),
                                        bincode::config::standard(),
                                    )
                                    && let Ok(request) = RequestBody::try_from(binary_req)
                                {
                                    let response_result = request_handler(request).await;

                                    if let Ok(response) = response_result {
                                        // Serialize response to binary
                                        if let Ok(binary_resp) =
                                            BinaryResponseBody::try_from(response)
                                            && let Ok(response_bytes) = bincode::encode_to_vec(
                                                &binary_resp,
                                                bincode::config::standard(),
                                            )
                                        {
                                            let mut conn = pub_connection_clone.clone();
                                            let _ = conn
                                                .publish::<_, _, ()>(
                                                    &response_channel_clone,
                                                    response_bytes,
                                                )
                                                .await;
                                        }
                                    }
                                }
                            } else if channel == response_channel_clone {
                                // Handle response message - deserialize from binary
                                if let Ok((binary_resp, _)) =
                                    bincode::decode_from_slice::<BinaryResponseBody, _>(
                                        payload.as_bytes(),
                                        bincode::config::standard(),
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
                    } else {
                        // Error getting payload - connection might be broken
                        warn!("Error getting message payload: {:?}", payload_result);
                        connection_broken = true;
                        break;
                    }
                }

                if connection_broken {
                    warn!(
                        "Pub/sub connection broken, reconnecting in {}ms...",
                        retry_delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay)).await;
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                } else {
                    warn!("Pub/sub message stream ended unexpectedly, reconnecting...");
                }

                // Connection ended, will retry in outer loop
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
                        let node_count = (count as usize).max(1);
                        debug!(
                            "Detected {} application instances via PUBSUB NUMSUB",
                            node_count
                        );
                        Ok(node_count)
                    } else {
                        warn!("PUBSUB NUMSUB returned non-integer count: {:?}", values[1]);
                        Ok(1)
                    }
                } else {
                    warn!("PUBSUB NUMSUB returned unexpected format: {:?}", values);
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
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| Error::Redis(format!("Failed to acquire health check connection: {e}")))?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| Error::Redis(format!("Health check PING failed: {e}")))?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(Error::Redis(format!(
                "PING returned unexpected response: {response}"
            )))
        }
    }
}
