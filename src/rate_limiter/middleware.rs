// src/rate_limiter/middleware.rs
use crate::rate_limiter::{RateLimitResult, RateLimiter};
use axum::{
    body::Body as AxumBody,
    extract::ConnectInfo,
    http::{HeaderMap, HeaderName, HeaderValue, Request as AxumRequest, StatusCode},
    response::{IntoResponse, Response as AxumResponse},
};
use futures_util::future::BoxFuture;
use hyper::Request as HyperRequest;
use sonic_rs::json;
use std::{
    fmt,
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
};
use tower_layer::Layer;
use tower_service::Service;
use tracing::{debug, error, warn};

#[derive(Debug)]
pub enum RateLimitMiddlewareError {
    InvalidHeaderName(String),
    ExtractionFailed(String),
}

impl fmt::Display for RateLimitMiddlewareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RateLimitMiddlewareError::InvalidHeaderName(e) => {
                write!(f, "Invalid header name for key extraction: {e}")
            }
            RateLimitMiddlewareError::ExtractionFailed(e) => {
                write!(f, "Key extraction failed: {e}")
            }
        }
    }
}

impl std::error::Error for RateLimitMiddlewareError {}

// Define header names as constants
const HEADER_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
const HEADER_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
const HEADER_RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");
const HEADER_RETRY_AFTER: HeaderName = HeaderName::from_static("retry-after");

#[derive(Debug, Clone)]
pub struct RateLimitOptions {
    pub include_headers: bool,
    pub fail_open: bool,
    pub key_prefix: Option<String>,
}

impl Default for RateLimitOptions {
    fn default() -> Self {
        Self {
            include_headers: true,
            fail_open: true,
            key_prefix: None,
        }
    }
}

#[derive(Clone)]
pub struct RateLimitLayer<K> {
    limiter: Arc<dyn RateLimiter>,
    key_extractor: Arc<K>,
    options: RateLimitOptions,
    metrics: Option<Arc<tokio::sync::Mutex<dyn crate::metrics::MetricsInterface + Send + Sync>>>,
    config_name: String, // Track which rate limit config this is using
}

impl<K> RateLimitLayer<K>
where
    K: KeyExtractor + Clone + Send + Sync + 'static,
{
    pub fn new(limiter: Arc<dyn RateLimiter>, key_extractor: K) -> Self {
        Self::with_options(limiter, key_extractor, RateLimitOptions::default())
    }

    #[allow(dead_code)]
    pub fn with_options(
        limiter: Arc<dyn RateLimiter>,
        key_extractor: K,
        options: RateLimitOptions,
    ) -> Self {
        Self {
            limiter,
            key_extractor: Arc::new(key_extractor),
            options,
            metrics: None,
            config_name: "unknown".to_string(),
        }
    }

    pub fn with_config_name(mut self, config_name: String) -> Self {
        self.config_name = config_name;
        self
    }

    pub fn with_metrics(
        mut self,
        metrics: Arc<tokio::sync::Mutex<dyn crate::metrics::MetricsInterface + Send + Sync>>,
    ) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

impl<S, K> Layer<S> for RateLimitLayer<K>
where
    S: Clone + Send + 'static,
    S: Service<AxumRequest<AxumBody>, Response = AxumResponse>,
    S::Future: Send + 'static,
    K: KeyExtractor + Clone + Send + Sync + 'static,
{
    type Service = RateLimitService<S, K>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: self.limiter.clone(),
            key_extractor: self.key_extractor.clone(),
            options: self.options.clone(),
            metrics: self.metrics.clone(),
            config_name: self.config_name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RateLimitService<S, K> {
    inner: S,
    limiter: Arc<dyn RateLimiter>,
    key_extractor: Arc<K>,
    options: RateLimitOptions,
    metrics: Option<Arc<tokio::sync::Mutex<dyn crate::metrics::MetricsInterface + Send + Sync>>>,
    config_name: String,
}

impl<S, K> Service<AxumRequest<AxumBody>> for RateLimitService<S, K>
where
    S: Clone + Send + 'static,
    S: Service<AxumRequest<AxumBody>, Response = AxumResponse>,
    S::Future: Send + 'static,
    S::Error: IntoResponse + Send,
    K: KeyExtractor + Send + Sync + 'static,
{
    type Response = AxumResponse;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: AxumRequest<AxumBody>) -> Self::Future {
        let limiter = self.limiter.clone();
        let key_extractor = self.key_extractor.clone();
        let options = self.options.clone();
        let metrics = self.metrics.clone();
        let config_name = self.config_name.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let key = match key_extractor.extract(&req) {
                Ok(k) => k,
                Err(e) => {
                    error!("Failed to extract key for rate limiting: {}", e);
                    return Ok(internal_server_error_response_with_message(
                        "Key extraction failed for rate limiting.",
                    ));
                }
            };

            debug!(key = %key, "Extracted rate limit key");

            let final_key = if let Some(prefix) = &options.key_prefix {
                format!("{prefix}:{key}")
            } else {
                key
            };
            debug!(final_key = %final_key, "Final rate limit key");

            // Use the config name as the primary limiter type
            let primary_limiter_type = &config_name;

            // Track request context for additional granularity
            let path = req.uri().path();
            let request_context = if path.starts_with("/app/") {
                "websocket_upgrade"
            } else if path.starts_with("/apps/") {
                "http_api"
            } else if path.starts_with("/up/") {
                "health_check"
            } else {
                "other"
            };

            // Track rate limit check with config name
            if let Some(ref metrics) = metrics {
                let metrics_locked = metrics.lock().await;
                // Use "global" as app_id for IP-based rate limiting
                metrics_locked.mark_rate_limit_check_with_context(
                    "global",
                    primary_limiter_type,
                    request_context,
                );
            }

            let rate_limit_result = match limiter.increment(&final_key).await {
                Ok(result) => result,
                Err(e) => {
                    error!("Rate limiter backend error for key '{}': {}", final_key, e);
                    if options.fail_open {
                        warn!("{}", "Rate limiter failed open");
                        RateLimitResult {
                            allowed: true,
                            remaining: 0,
                            reset_after: 0,
                            limit: 0,
                        }
                    } else {
                        error!(key = %final_key, "Rate limiter failed closed");
                        return Ok(internal_server_error_response_with_message(
                            "Rate limiter backend unavailable.",
                        ));
                    }
                }
            };

            if !rate_limit_result.allowed {
                debug!(key = %final_key, "Rate limit exceeded for config: {}", config_name);

                // Track rate limit triggered with config name
                if let Some(ref metrics) = metrics {
                    let metrics_locked = metrics.lock().await;
                    metrics_locked.mark_rate_limit_triggered_with_context(
                        "global",
                        primary_limiter_type,
                        request_context,
                    );
                }

                return Ok(rate_limit_error_response(Some(&rate_limit_result)));
            }

            debug!(key = %final_key, "Rate limit check passed");
            let result = inner.call(req).await;

            match result {
                Ok(mut response) => {
                    // Only add headers if include_headers is true AND the limiter didn't fail open with dummy values
                    if options.include_headers && rate_limit_result.limit > 0 {
                        add_rate_limit_headers(response.headers_mut(), &rate_limit_result, false);
                    }
                    Ok(response)
                }
                Err(err) => Err(err),
            }
        })
    }
}

// --- Key Extractors ---

pub trait KeyExtractor: Send + Sync {
    fn extract<B>(&self, req: &HyperRequest<B>) -> Result<String, RateLimitMiddlewareError>;
}

#[derive(Clone, Debug)]
pub struct IpKeyExtractor {
    trust_hops: usize,
}

impl IpKeyExtractor {
    pub fn new(trust_hops: usize) -> Self {
        Self { trust_hops }
    }

    fn get_ip<B>(&self, req: &HyperRequest<B>) -> Option<String> {
        if self.trust_hops > 0
            && let Some(value) = req.headers().get("x-forwarded-for")
            && let Ok(forwarded_str) = value.to_str()
        {
            let ips: Vec<&str> = forwarded_str.split(',').map(str::trim).collect();
            let client_ip_index = ips.len().saturating_sub(self.trust_hops);
            if let Some(ip_str) = ips.get(client_ip_index) {
                if ip_str.parse::<std::net::IpAddr>().is_ok() {
                    return Some(ip_str.to_string());
                }
            } else if let Some(ip_str) = ips.first()
                && ip_str.parse::<std::net::IpAddr>().is_ok()
            {
                return Some(ip_str.to_string());
            }
        }

        if let Some(value) = req.headers().get("x-real-ip")
            && let Ok(real_ip_str) = value.to_str()
        {
            let real_ip = real_ip_str.trim();
            if real_ip.parse::<std::net::IpAddr>().is_ok() {
                return Some(real_ip.to_string());
            }
        }

        req.extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip().to_string())
    }
}

impl Default for IpKeyExtractor {
    fn default() -> Self {
        Self::new(0)
    }
}

impl KeyExtractor for IpKeyExtractor {
    fn extract<B>(&self, req: &HyperRequest<B>) -> Result<String, RateLimitMiddlewareError> {
        Ok(self.get_ip(req).unwrap_or_else(|| {
            warn!(
                "{}",
                "Could not extract IP address for rate limiting, falling back to 'unknown_ip'"
            );
            "unknown_ip".to_string()
        }))
    }
}

// --- Helper Functions ---

fn rate_limit_error_response(result: Option<&RateLimitResult>) -> AxumResponse {
    let mut response = axum::response::Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(AxumBody::from(
            sonic_rs::to_string(&json!({
                "error": "Too Many Requests",
                "message": "Rate limit exceeded. Please try again later.",
            }))
            .expect("Failed to serialize rate limit error response"),
        ))
        .expect("Failed to build rate limit error response");

    // For 429 responses, always include headers if result is available
    if let Some(res) = result {
        add_rate_limit_headers(response.headers_mut(), res, true);
    }
    response
}

fn internal_server_error_response_with_message(message: &str) -> AxumResponse {
    axum::response::Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(AxumBody::from(
            sonic_rs::to_string(&json!({
                "error": "Internal Server Error",
                "message": message,
            }))
            .expect("Failed to serialize internal server error response"),
        ))
        .expect("Failed to build internal server error response")
}

// Updated add_rate_limit_headers as per user's working version
fn add_rate_limit_headers(
    headers: &mut HeaderMap,
    result: &RateLimitResult,
    is_rate_limited: bool,
) {
    if let Ok(value) = HeaderValue::try_from(result.limit.to_string()) {
        headers.insert(HEADER_LIMIT, value);
    } else {
        warn!(
            value = result.limit,
            "Failed to convert rate limit limit value for header X-RateLimit-Limit"
        );
    }

    // Conditionally add X-RateLimit-Remaining and X-RateLimit-Reset if they were not the cause of the panic
    // For now, let's include them as per a standard implementation, but be mindful if panic returns.
    if let Ok(value) = HeaderValue::try_from(result.remaining.to_string()) {
        headers.insert(HEADER_REMAINING, value);
    } else {
        warn!(
            value = result.remaining,
            "Failed to convert rate limit remaining value for header X-RateLimit-Remaining"
        );
    }

    if let Ok(value) = HeaderValue::try_from(result.reset_after.to_string()) {
        headers.insert(HEADER_RESET, value.clone()); // Clone for Retry-After
        if is_rate_limited {
            headers.insert(HEADER_RETRY_AFTER, value);
        }
    } else {
        warn!(
            value = result.reset_after,
            "Failed to convert rate limit reset_after value for header X-RateLimit-Reset/Retry-After"
        );
    }
}
