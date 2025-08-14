use super::WebhookEvent;
use crate::webhook::integration::WebhookIntegration;
use std::sync::Arc;
use tracing::{debug, error};

pub struct AsyncWebhookProcessor {
    webhook_integration: Arc<WebhookIntegration>,
}

impl AsyncWebhookProcessor {
    pub fn new(webhook_integration: Arc<WebhookIntegration>) -> Self {
        Self {
            webhook_integration,
        }
    }

    pub async fn process_events(&self, events: Vec<WebhookEvent>) {
        if events.is_empty() {
            return;
        }

        debug!("Processing {} webhook events asynchronously", events.len());

        let webhook_tasks: Vec<_> = events
            .into_iter()
            .map(|event| {
                let webhook = self.webhook_integration.clone();
                tokio::spawn(async move {
                    if let Err(e) = Self::send_webhook_event(&webhook, event).await {
                        error!("Failed to send webhook event: {}", e);
                    }
                })
            })
            .collect();

        // Spawn a task to handle all webhook deliveries without blocking
        tokio::spawn(async move {
            for task in webhook_tasks {
                if let Err(e) = task.await {
                    error!("Webhook task panicked: {}", e);
                }
            }
        });
    }

    async fn send_webhook_event(
        webhook_integration: &WebhookIntegration,
        event: WebhookEvent,
    ) -> crate::error::Result<()> {
        match event.event_type.as_str() {
            "member_removed" => {
                // Note: This would need to integrate with the actual webhook integration
                // For now, we'll just log the event
                debug!(
                    "Would send member_removed webhook for user {} in channel {}",
                    event.user_id.as_deref().unwrap_or("unknown"),
                    event.channel
                );
            }
            "channel_vacated" => {
                debug!(
                    "Would send channel_vacated webhook for channel {}",
                    event.channel
                );
            }
            _ => {
                debug!("Would send {} webhook event", event.event_type);
            }
        }

        Ok(())
    }
}
