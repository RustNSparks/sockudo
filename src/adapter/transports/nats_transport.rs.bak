use crate::adapter::binary_protocol::{
    BinaryBroadcastMessage, BinaryRequestBody, BinaryResponseBody,
};
use crate::adapter::horizontal_adapter::{BroadcastMessage, RequestBody, ResponseBody};
use crate::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use crate::error::{Error, Result};
use crate::options::NatsAdapterConfig;
use async_nats::{Client as NatsClient, ConnectOptions as NatsOptions, Subject};
use async_trait::async_trait;
use futures::StreamExt;
use std::time::Duration;
use tracing::{debug, info, warn};

/// NATS transport implementation
#[derive(Clone)]
pub struct NatsTransport {
    client: NatsClient,
    broadcast_subject: String,
    request_subject: String,
    response_subject: String,
    config: NatsAdapterConfig,
}

impl TransportConfig for NatsAdapterConfig {
    fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    fn prefix(&self) -> &str {
        &self.prefix
    }
}

#[async_trait]
impl HorizontalTransport for NatsTransport {
    type Config = NatsAdapterConfig;

    async fn new(config: Self::Config) -> Result<Self> {
        info!(
            "NATS transport config: servers={:?}, prefix={}, request_timeout={}ms, connection_timeout={}ms",
            config.servers, config.prefix, config.request_timeout_ms, config.connection_timeout_ms
        );
        debug!(
            "NATS transport credentials: username={:?}, password={:?}, token={:?}",
            config.username, config.password, config.token
        );

        // Build NATS Options
        let mut nats_options = NatsOptions::new();

        // Set credentials conditionally
        if let (Some(username), Some(password)) =
            (config.username.as_deref(), config.password.as_deref())
        {
            nats_options =
                nats_options.user_and_password(username.to_string(), password.to_string());
        } else if let Some(token) = config.token.as_deref() {
            nats_options = nats_options.token(token.to_string());
        }

        // Set connection timeout
        nats_options =
            nats_options.connection_timeout(Duration::from_millis(config.connection_timeout_ms));

        // Connect to NATS
        let client = nats_options
            .connect(&config.servers)
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to NATS: {e}")))?;

        // Build subject names
        let broadcast_subject = format!("{}.broadcast", config.prefix);
        let request_subject = format!("{}.requests", config.prefix);
        let response_subject = format!("{}.responses", config.prefix);

        Ok(Self {
            client,
            broadcast_subject,
            request_subject,
            response_subject,
            config,
        })
    }

    async fn publish_broadcast(&self, message: &BroadcastMessage) -> Result<()> {
        // Convert to binary format
        let binary_msg: BinaryBroadcastMessage = message.clone().into();
        let message_data = bincode::serialize(&binary_msg)
            .map_err(|e| Error::Other(format!("Failed to serialize broadcast: {}", e)))?;

        self.client
            .publish(
                Subject::from(self.broadcast_subject.clone()),
                message_data.into(),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to publish broadcast: {e}")))?;

        debug!("Published broadcast message via NATS");
        Ok(())
    }

    async fn publish_request(&self, request: &RequestBody) -> Result<()> {
        // Convert to binary format
        let binary_req: BinaryRequestBody = request.clone().try_into()?;
        let request_data = bincode::serialize(&binary_req)
            .map_err(|e| Error::Other(format!("Failed to serialize request: {}", e)))?;

        self.client
            .publish(
                Subject::from(self.request_subject.clone()),
                request_data.into(),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to publish request: {e}")))?;

        debug!("Broadcasted request {} via NATS", request.request_id);
        Ok(())
    }

    async fn publish_response(&self, response: &ResponseBody) -> Result<()> {
        // Convert to binary format
        let binary_resp: BinaryResponseBody = response.clone().try_into()?;
        let response_data = bincode::serialize(&binary_resp)
            .map_err(|e| Error::Other(format!("Failed to serialize response: {}", e)))?;

        self.client
            .publish(
                Subject::from(self.response_subject.clone()),
                response_data.into(),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to publish response: {e}")))?;

        debug!("Published response via NATS");
        Ok(())
    }

    async fn start_listeners(&self, handlers: TransportHandlers) -> Result<()> {
        let client = self.client.clone();
        let broadcast_subject = self.broadcast_subject.clone();
        let request_subject = self.request_subject.clone();
        let response_subject = self.response_subject.clone();
        let response_client = self.client.clone();

        // Subscribe to broadcast channel
        let mut broadcast_subscription = client
            .subscribe(Subject::from(broadcast_subject.clone()))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to subscribe to broadcast subject: {e}"))
            })?;

        // Subscribe to requests channel
        let mut request_subscription = client
            .subscribe(Subject::from(request_subject.clone()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to subscribe to request subject: {e}")))?;

        // Subscribe to responses channel
        let mut response_subscription = client
            .subscribe(Subject::from(response_subject.clone()))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to subscribe to response subject: {e}"))
            })?;

        info!(
            "NATS transport listening on subjects: {}, {}, {}",
            broadcast_subject, request_subject, response_subject
        );

        // Spawn a task to handle broadcast messages
        let broadcast_handler = handlers.on_broadcast.clone();
        tokio::spawn(async move {
            while let Some(msg) = broadcast_subscription.next().await {
                if let Ok(binary_msg) = bincode::deserialize::<BinaryBroadcastMessage>(&msg.payload)
                {
                    let broadcast: BroadcastMessage = binary_msg.into();
                    broadcast_handler(broadcast).await;
                }
            }
        });

        // Spawn a task to handle request messages
        let request_handler = handlers.on_request.clone();
        tokio::spawn(async move {
            while let Some(msg) = request_subscription.next().await {
                if let Ok(binary_req) = bincode::deserialize::<BinaryRequestBody>(&msg.payload) {
                    if let Ok(request) = RequestBody::try_from(binary_req) {
                        let response_result = request_handler(request).await;

                        if let Ok(response) = response_result {
                            // Serialize response to binary
                            if let Ok(binary_resp) = BinaryResponseBody::try_from(response) {
                                if let Ok(response_data) = bincode::serialize(&binary_resp) {
                                    let _ = response_client
                                        .publish(
                                            Subject::from(response_subject.clone()),
                                            response_data.into(),
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                }
            }
        });

        // Spawn a task to handle response messages
        let response_handler = handlers.on_response.clone();
        tokio::spawn(async move {
            while let Some(msg) = response_subscription.next().await {
                if let Ok(binary_resp) = bincode::deserialize::<BinaryResponseBody>(&msg.payload) {
                    if let Ok(response) = ResponseBody::try_from(binary_resp) {
                        response_handler(response).await;
                    }
                } else {
                    warn!("Failed to parse binary response message");
                }
            }
        });

        Ok(())
    }

    async fn get_node_count(&self) -> Result<usize> {
        // If nodes_number is explicitly set, use that value
        if let Some(nodes) = self.config.nodes_number {
            return Ok(nodes as usize);
        }

        // NATS doesn't provide an easy way to count subscriptions like Redis
        // For now, we'll assume at least 1 node (ourselves)
        Ok(1)
    }

    async fn check_health(&self) -> Result<()> {
        // Check if the NATS client connection is still active
        // NATS client maintains internal health state
        let state = self.client.connection_state();
        match state {
            async_nats::connection::State::Connected => Ok(()),
            async_nats::connection::State::Disconnected => Err(crate::error::Error::Connection(
                "NATS client is disconnected".to_string(),
            )),
            other_state => Err(crate::error::Error::Connection(format!(
                "NATS client in transitional state: {other_state:?}"
            ))),
        }
    }
}
