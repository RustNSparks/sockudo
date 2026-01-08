#![allow(dead_code)]

// src/metrics/prometheus.rs

use crate::error::Result;

use super::MetricsInterface;
use crate::websocket::SocketId;
use async_trait::async_trait;
use prometheus::{
    CounterVec, GaugeVec, HistogramVec, Opts, TextEncoder, histogram_opts, register_counter_vec,
    register_gauge_vec, register_histogram_vec,
};
use serde_json::{Value, json};
use tracing::{debug, error};

// Histogram buckets for internal operations (in milliseconds)
// Optimized for sub-millisecond to low-millisecond measurements
const INTERNAL_LATENCY_HISTOGRAM_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

// Histogram buckets for end-to-end operations (in milliseconds)
// Covers typical response times and high-latency scenarios
const END_TO_END_LATENCY_HISTOGRAM_BUCKETS: &[f64] = &[
    0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
];

/// A Prometheus implementation of the metrics interface
pub struct PrometheusMetricsDriver {
    prefix: String,
    port: u16,

    // Metrics
    connected_sockets: GaugeVec,
    new_connections_total: CounterVec,
    new_disconnections_total: CounterVec,
    connection_errors_total: CounterVec,
    socket_bytes_received: CounterVec,
    socket_bytes_transmitted: CounterVec,
    ws_messages_received: CounterVec,
    ws_messages_sent: CounterVec,
    http_bytes_received: CounterVec,
    http_bytes_transmitted: CounterVec,
    http_calls_received: CounterVec,
    horizontal_adapter_resolve_time: HistogramVec,
    horizontal_adapter_resolved_promises: CounterVec,
    horizontal_adapter_uncomplete_promises: CounterVec,
    horizontal_adapter_sent_requests: CounterVec,
    horizontal_adapter_received_requests: CounterVec,
    horizontal_adapter_received_responses: CounterVec,
    rate_limit_checks_total: CounterVec,
    rate_limit_triggered_total: CounterVec,
    channel_subscriptions_total: CounterVec,
    channel_unsubscriptions_total: CounterVec,
    active_channels: GaugeVec,
    broadcast_latency_ms: HistogramVec,
}

impl PrometheusMetricsDriver {
    /// Creates a new Prometheus metrics driver
    pub async fn new(port: u16, prefix_opt: Option<&str>) -> Self {
        let prefix = prefix_opt.unwrap_or("sockudo_").to_string();

        // Initialize all metrics
        let connected_sockets = register_gauge_vec!(
            Opts::new(
                format!("{prefix}connected"),
                "The number of currently connected sockets"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let new_connections_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}new_connections_total"),
                "Total amount of sockudo connection requests"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let new_disconnections_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}new_disconnections_total"),
                "Total amount of sockudo disconnections"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let connection_errors_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}connection_errors_total"),
                "Total amount of connection errors by type"
            ),
            &["app_id", "port", "error_type"]
        )
        .unwrap();

        let socket_bytes_received = register_counter_vec!(
            Opts::new(
                format!("{prefix}socket_received_bytes"),
                "Total amount of bytes that sockudo received"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let socket_bytes_transmitted = register_counter_vec!(
            Opts::new(
                format!("{prefix}socket_transmitted_bytes"),
                "Total amount of bytes that sockudo transmitted"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let ws_messages_received = register_counter_vec!(
            Opts::new(
                format!("{prefix}ws_messages_received_total"),
                "The total amount of WS messages received from connections by the server"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let ws_messages_sent = register_counter_vec!(
            Opts::new(
                format!("{prefix}ws_messages_sent_total"),
                "The total amount of WS messages sent to the connections from the server"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let http_bytes_received = register_counter_vec!(
            Opts::new(
                format!("{prefix}http_received_bytes"),
                "Total amount of bytes that sockudo's REST API received"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let http_bytes_transmitted = register_counter_vec!(
            Opts::new(
                format!("{prefix}http_transmitted_bytes"),
                "Total amount of bytes that sockudo's REST API sent back"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let http_calls_received = register_counter_vec!(
            Opts::new(
                format!("{prefix}http_calls_received_total"),
                "Total amount of received REST API calls"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_resolve_time = register_histogram_vec!(
            histogram_opts!(
                format!("{}horizontal_adapter_resolve_time", prefix),
                "The average resolve time for requests to other nodes",
                INTERNAL_LATENCY_HISTOGRAM_BUCKETS.to_vec()
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_resolved_promises = register_counter_vec!(
            Opts::new(
                format!("{prefix}horizontal_adapter_resolved_promises"),
                "The total amount of promises that were fulfilled by other nodes"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_uncomplete_promises = register_counter_vec!(
            Opts::new(
                format!("{prefix}horizontal_adapter_uncomplete_promises"),
                "The total amount of promises that were not fulfilled entirely by other nodes"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_sent_requests = register_counter_vec!(
            Opts::new(
                format!("{prefix}horizontal_adapter_sent_requests"),
                "The total amount of sent requests to other nodes"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_received_requests = register_counter_vec!(
            Opts::new(
                format!("{prefix}horizontal_adapter_received_requests"),
                "The total amount of received requests from other nodes"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let horizontal_adapter_received_responses = register_counter_vec!(
            Opts::new(
                format!("{prefix}horizontal_adapter_received_responses"),
                "The total amount of received responses from other nodes"
            ),
            &["app_id", "port"]
        )
        .unwrap();

        let rate_limit_checks_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}rate_limit_checks_total"),
                "Total number of rate limit checks performed"
            ),
            &["app_id", "port", "limiter_type", "request_context"]
        )
        .unwrap();

        let rate_limit_triggered_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}rate_limit_triggered_total"),
                "Total number of times rate limit was triggered"
            ),
            &["app_id", "port", "limiter_type", "request_context"]
        )
        .unwrap();

        let channel_subscriptions_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}channel_subscriptions_total"),
                "Total number of channel subscriptions"
            ),
            &["app_id", "port", "channel_type"]
        )
        .unwrap();

        let channel_unsubscriptions_total = register_counter_vec!(
            Opts::new(
                format!("{prefix}channel_unsubscriptions_total"),
                "Total number of channel unsubscriptions"
            ),
            &["app_id", "port", "channel_type"]
        )
        .unwrap();

        let active_channels = register_gauge_vec!(
            Opts::new(
                format!("{prefix}active_channels"),
                "Number of currently active channels"
            ),
            &["app_id", "port", "channel_type"]
        )
        .unwrap();

        let broadcast_latency_ms = register_histogram_vec!(
            histogram_opts!(
                format!("{prefix}broadcast_latency_ms"),
                "End-to-end latency for broadcast messages in milliseconds",
                END_TO_END_LATENCY_HISTOGRAM_BUCKETS.to_vec()
            ),
            &["app_id", "port", "channel_type", "recipient_count_bucket"]
        )
        .unwrap();

        // Reset gauge metrics to 0 on startup - they represent current state, not historical
        connected_sockets.reset();
        active_channels.reset();

        Self {
            prefix,
            port,
            connected_sockets,
            new_connections_total,
            new_disconnections_total,
            connection_errors_total,
            socket_bytes_received,
            socket_bytes_transmitted,
            ws_messages_received,
            ws_messages_sent,
            http_bytes_received,
            http_bytes_transmitted,
            http_calls_received,
            horizontal_adapter_resolve_time,
            horizontal_adapter_resolved_promises,
            horizontal_adapter_uncomplete_promises,
            horizontal_adapter_sent_requests,
            horizontal_adapter_received_requests,
            horizontal_adapter_received_responses,
            rate_limit_checks_total,
            rate_limit_triggered_total,
            channel_subscriptions_total,
            channel_unsubscriptions_total,
            active_channels,
            broadcast_latency_ms,
        }
    }

    /// Get the tags for Prometheus
    fn get_tags(&self, app_id: &str) -> Vec<String> {
        vec![app_id.to_string(), self.port.to_string()]
    }
}

#[async_trait]
impl MetricsInterface for PrometheusMetricsDriver {
    async fn init(&self) -> Result<()> {
        // Not needed for Prometheus, metrics are registered in the constructor
        Ok(())
    }

    fn mark_new_connection(&self, app_id: &str, socket_id: &SocketId) {
        let tags = self.get_tags(app_id);
        self.connected_sockets.with_label_values(&tags).inc();
        self.new_connections_total.with_label_values(&tags).inc();

        debug!(
            "Metrics: New connection for app {}, socket {}",
            app_id, socket_id
        );
    }

    fn mark_disconnection(&self, app_id: &str, socket_id: &SocketId) {
        let tags = self.get_tags(app_id);
        self.connected_sockets.with_label_values(&tags).dec();
        self.new_disconnections_total.with_label_values(&tags).inc();

        debug!(
            "Metrics: Disconnection for app {}, socket {}",
            app_id, socket_id
        );
    }

    fn mark_connection_error(&self, app_id: &str, error_type: &str) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            error_type.to_string(),
        ];
        self.connection_errors_total.with_label_values(&tags).inc();

        debug!(
            "Metrics: Connection error for app {}, error type: {}",
            app_id, error_type
        );
    }

    fn mark_rate_limit_check(&self, app_id: &str, limiter_type: &str) {
        self.mark_rate_limit_check_with_context(app_id, limiter_type, "unknown");
    }

    fn mark_rate_limit_check_with_context(
        &self,
        app_id: &str,
        limiter_type: &str,
        request_context: &str,
    ) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            limiter_type.to_string(),
            request_context.to_string(),
        ];
        self.rate_limit_checks_total.with_label_values(&tags).inc();

        debug!(
            "Metrics: Rate limit check for app {}, limiter type: {}, context: {}",
            app_id, limiter_type, request_context
        );
    }

    fn mark_rate_limit_triggered(&self, app_id: &str, limiter_type: &str) {
        self.mark_rate_limit_triggered_with_context(app_id, limiter_type, "unknown");
    }

    fn mark_rate_limit_triggered_with_context(
        &self,
        app_id: &str,
        limiter_type: &str,
        request_context: &str,
    ) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            limiter_type.to_string(),
            request_context.to_string(),
        ];
        self.rate_limit_triggered_total
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Rate limit triggered for app {}, limiter type: {}, context: {}",
            app_id, limiter_type, request_context
        );
    }

    fn mark_channel_subscription(&self, app_id: &str, channel_type: &str) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            channel_type.to_string(),
        ];
        self.channel_subscriptions_total
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Channel subscription for app {}, channel type: {}",
            app_id, channel_type
        );
    }

    fn mark_channel_unsubscription(&self, app_id: &str, channel_type: &str) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            channel_type.to_string(),
        ];
        self.channel_unsubscriptions_total
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Channel unsubscription for app {}, channel type: {}",
            app_id, channel_type
        );
    }

    fn update_active_channels(&self, app_id: &str, channel_type: &str, count: i64) {
        let tags = vec![
            app_id.to_string(),
            self.port.to_string(),
            channel_type.to_string(),
        ];
        self.active_channels
            .with_label_values(&tags)
            .set(count as f64);

        debug!(
            "Metrics: Active channels updated for app {}, channel type: {}, count: {}",
            app_id, channel_type, count
        );
    }

    fn mark_api_message(
        &self,
        app_id: &str,
        incoming_message_size: usize,
        sent_message_size: usize,
    ) {
        let tags = self.get_tags(app_id);
        self.http_bytes_received
            .with_label_values(&tags)
            .inc_by(incoming_message_size as f64);
        self.http_bytes_transmitted
            .with_label_values(&tags)
            .inc_by(sent_message_size as f64);
        self.http_calls_received.with_label_values(&tags).inc();

        debug!(
            "Metrics: API message for app {}, incoming size: {}, sent size: {}",
            app_id, incoming_message_size, sent_message_size
        );
    }

    fn mark_ws_message_sent(&self, app_id: &str, sent_message_size: usize) {
        let tags = self.get_tags(app_id);
        self.socket_bytes_transmitted
            .with_label_values(&tags)
            .inc_by(sent_message_size as f64);
        self.ws_messages_sent.with_label_values(&tags).inc();

        debug!(
            "Metrics: WS message sent for app {}, size: {}",
            app_id, sent_message_size
        );
    }

    fn mark_ws_messages_sent_batch(&self, app_id: &str, sent_message_size: usize, count: usize) {
        let tags = self.get_tags(app_id);
        // Batch update: total bytes = message_size * count, total messages = count
        self.socket_bytes_transmitted
            .with_label_values(&tags)
            .inc_by((sent_message_size * count) as f64);
        self.ws_messages_sent
            .with_label_values(&tags)
            .inc_by(count as f64);

        debug!(
            "Metrics: WS messages sent batch for app {}, count: {}, total size: {}",
            app_id,
            count,
            sent_message_size * count
        );
    }

    fn mark_ws_message_received(&self, app_id: &str, message_size: usize) {
        let tags = self.get_tags(app_id);
        self.socket_bytes_received
            .with_label_values(&tags)
            .inc_by(message_size as f64);
        self.ws_messages_received.with_label_values(&tags).inc();

        debug!(
            "Metrics: WS message received for app {}, size: {}",
            app_id, message_size
        );
    }

    fn track_horizontal_adapter_resolve_time(&self, app_id: &str, time_ms: f64) {
        let tags = self.get_tags(app_id);
        self.horizontal_adapter_resolve_time
            .with_label_values(&tags)
            .observe(time_ms);

        debug!(
            "Metrics: Horizontal adapter resolve time for app {}, time: {} ms",
            app_id, time_ms
        );
    }

    fn track_horizontal_adapter_resolved_promises(&self, app_id: &str, resolved: bool) {
        let tags = self.get_tags(app_id);

        if resolved {
            self.horizontal_adapter_resolved_promises
                .with_label_values(&tags)
                .inc();
        } else {
            self.horizontal_adapter_uncomplete_promises
                .with_label_values(&tags)
                .inc();
        }

        debug!(
            "Metrics: Horizontal adapter promise {} for app {}",
            if resolved { "resolved" } else { "unresolved" },
            app_id
        );
    }

    fn mark_horizontal_adapter_request_sent(&self, app_id: &str) {
        let tags = self.get_tags(app_id);
        self.horizontal_adapter_sent_requests
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Horizontal adapter request sent for app {}",
            app_id
        );
    }

    fn mark_horizontal_adapter_request_received(&self, app_id: &str) {
        let tags = self.get_tags(app_id);
        self.horizontal_adapter_received_requests
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Horizontal adapter request received for app {}",
            app_id
        );
    }

    fn mark_horizontal_adapter_response_received(&self, app_id: &str) {
        let tags = self.get_tags(app_id);
        self.horizontal_adapter_received_responses
            .with_label_values(&tags)
            .inc();

        debug!(
            "Metrics: Horizontal adapter response received for app {}",
            app_id
        );
    }

    fn track_broadcast_latency(
        &self,
        app_id: &str,
        channel_name: &str,
        recipient_count: usize,
        latency_ms: f64,
    ) {
        // Determine channel type from channel name using the ChannelType enum
        let channel_type = crate::channel::ChannelType::from_name(channel_name).as_str();

        if recipient_count == 0 {
            return;
        }

        // Determine recipient count bucket
        let bucket = match recipient_count {
            1..=10 => "xs",
            11..=100 => "sm",
            101..=1000 => "md",
            1001..=10000 => "lg",
            _ => "xl",
        };

        self.broadcast_latency_ms
            .with_label_values(&[app_id, &self.port.to_string(), channel_type, bucket])
            .observe(latency_ms);

        debug!(
            "Metrics: Broadcast latency for app {}, channel: {} ({}), recipients: {} ({}), latency: {} ms",
            app_id, channel_name, channel_type, recipient_count, bucket, latency_ms
        );
    }

    async fn get_metrics_as_plaintext(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = prometheus::gather(); // Gather from the default registry
        encoder
            .encode_to_string(&metric_families)
            .unwrap_or_else(|e| {
                error!("{}", format!("Failed to encode metrics to string: {}", e));
                String::from("Error encoding metrics")
            })
    }

    /// Get metrics data as a JSON object
    async fn get_metrics_as_json(&self) -> Value {
        // Create a base JSON structure
        let metrics_json = json!({
            "connections": {
                "total": 0,
                "current": 0
            },
            "messages": {
                "received": {
                    "total": 0,
                    "bytes": 0
                },
                "transmitted": {
                    "total": 0,
                    "bytes": 0
                }
            },
            "api": {
                "requests": {
                    "total": 0,
                    "received_bytes": 0,
                    "transmitted_bytes": 0
                }
            },
            "horizontal_adapter": {
                "requests": {
                    "sent": 0,
                    "received": 0,
                    "responses_received": 0
                },
                "promises": {
                    "resolved": 0,
                    "unresolved": 0
                },
                "resolve_time_ms": {
                    "avg": 0.0,
                    "max": 0.0,
                    "min": 0.0
                }
            },
            "channels": {
                "total": 0
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Gather metrics from Prometheus
        let metric_families = prometheus::gather();

        // Convert Prometheus metrics to JSON
        let mut json_metrics = json!({});

        for mf in metric_families {
            let name = mf.name();
            let metric_type = mf.type_();

            for m in mf.get_metric() {
                let labels = m.get_label();
                let mut label_json = json!({});

                // Process labels
                for label in labels {
                    label_json[label.name()] = json!(label.value());
                }

                // Get metric value based on type
                let value = match metric_type {
                    prometheus::proto::MetricType::COUNTER => {
                        json!(m.get_counter().value())
                    }
                    prometheus::proto::MetricType::GAUGE => {
                        json!(m.get_gauge().value())
                    }
                    prometheus::proto::MetricType::HISTOGRAM => {
                        let h = m.get_histogram();
                        let mut buckets = Vec::new();
                        for b in h.get_bucket() {
                            buckets.push(json!({
                                "upper_bound": b.upper_bound(),
                                "cumulative_count": b.cumulative_count()
                            }));
                        }
                        json!({
                            "sample_count": h.get_sample_count(),
                            "sample_sum": h.get_sample_sum(),
                            "buckets": buckets
                        })
                    }
                    _ => {
                        json!(null)
                    }
                };

                // Add to json_metrics
                if labels.is_empty() {
                    json_metrics[name] = value;
                } else {
                    if !json_metrics.as_object().unwrap().contains_key(name) {
                        json_metrics[name] = json!([]);
                    }
                    let mut metric_with_labels = label_json;
                    metric_with_labels["value"] = value;
                    json_metrics[name]
                        .as_array_mut()
                        .unwrap()
                        .push(metric_with_labels);
                }
            }
        }

        // Return raw metrics JSON for maximum flexibility
        json!({
            "formatted": metrics_json,
            "raw": json_metrics,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })
    }

    async fn clear(&self) {
        // Reset individual metrics counters - not fully supported by Prometheus Rust client
        // So we'll just log a message
        debug!("Metrics cleared (note: Prometheus metrics can't be fully cleared)");
    }
}
