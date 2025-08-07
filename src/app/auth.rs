use super::manager::AppManager;
use crate::app::config::App;
use crate::error::Error;
use crate::http_handler::EventQuery; // Assuming EventQuery is in http_handler.rs
use crate::token::{Token, secure_compare};
use crate::websocket::SocketId;
use chrono::Utc; // For timestamp validation
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct ChannelAuth {
    pub channel_name: String,
    pub socket_id: String,
    #[serde(default)]
    pub user_data: Option<String>,
}

pub struct AuthValidator {
    app_manager: Arc<dyn AppManager>,
}

#[derive(Debug)]
pub struct AuthValidationResult {
    pub is_valid: bool,
    pub signature: String,
}

impl AuthValidator {
    pub fn new(app_manager: Arc<dyn AppManager>) -> Self {
        AuthValidator { app_manager }
    }

    pub async fn validate_channel_auth(
        &self,
        socket_id: SocketId,
        app_key: &str,
        user_data: &str,
        auth: &str,
    ) -> Result<bool, Error> {
        let app = self.app_manager.find_by_id(app_key).await?;
        if app.is_none() {
            return Err(Error::InvalidAppKey);
        }
        let is_valid =
            self.sign_in_token_is_valid(socket_id.0.as_str(), user_data, auth, app.unwrap());
        Ok(is_valid)
    }

    /// Validates a Pusher-compatible API request signature.
    ///
    /// - `auth_params_from_query_struct`: Parsed essential authentication query parameters (auth_key, auth_timestamp, auth_signature).
    /// - `http_method`: The HTTP method (e.g., "GET", "POST").
    /// - `request_path`: The full request path (e.g., "/apps/your_app_id/events").
    /// - `all_query_params_from_url`: A map of ALL query parameters from the request URL, excluding `auth_signature`.
    /// - `request_body_bytes_for_md5_check`: Raw body bytes for POST MD5 check. For GET, this is None.
    pub async fn validate_pusher_api_request(
        &self,
        auth_params_from_query_struct: &EventQuery, // Used for auth_key, auth_timestamp, auth_signature
        http_method: &str,
        request_path: &str,
        all_query_params_from_url: &BTreeMap<String, String>, // Contains all query params from URL (except auth_signature)
        request_body_bytes_for_md5_check: Option<&[u8]>,
    ) -> Result<bool, Error> {
        // Fetch app using auth_key from the parsed EventQuery struct
        let app = self
            .app_manager
            .find_by_key(&auth_params_from_query_struct.auth_key)
            .await?;

        if app.is_none() {
            debug!(
                "App not found for key: {}",
                auth_params_from_query_struct.auth_key
            );
            return Err(Error::InvalidAppKey);
        }
        let app_config = app.unwrap();

        // --- Timestamp Validation ---
        let auth_ts_str = &auth_params_from_query_struct.auth_timestamp;
        if auth_ts_str.is_empty() {
            debug!("auth_timestamp is missing from query parameters.");
            return Err(Error::Auth("auth_timestamp is missing".to_string()));
        }
        let auth_ts: i64 = match auth_ts_str.parse() {
            Ok(ts) => ts,
            Err(_) => {
                debug!("Invalid auth_timestamp format: {}", auth_ts_str);
                return Err(Error::Auth("Invalid auth_timestamp format".to_string()));
            }
        };

        let current_ts = Utc::now().timestamp();
        if (current_ts - auth_ts).abs() > 600 {
            // 600 seconds = 10 minutes
            debug!(
                "Timestamp validation failed. Server time: {}, Provided timestamp: {}, Difference: {}s",
                current_ts,
                auth_ts,
                (current_ts - auth_ts).abs()
            );
            return Err(Error::Auth(
                "Timestamp expired or too far in the future".to_string(),
            ));
        }

        // --- Prepare parameters for signing string construction ---
        // Start with a copy of all query parameters received in the URL (excluding auth_signature)
        let params_for_signing_string: BTreeMap<String, String> = all_query_params_from_url.clone();

        // --- body_md5 validation and inclusion in signature params ---
        let uppercased_http_method = http_method.to_uppercase();
        if uppercased_http_method == "POST" {
            if let Some(body_bytes) = request_body_bytes_for_md5_check {
                // POST with a non-empty body
                match params_for_signing_string.get("body_md5") {
                    Some(body_md5_from_query) if !body_md5_from_query.is_empty() => {
                        let actual_body_md5 = format!("{:x}", md5::compute(body_bytes));
                        if !secure_compare(&actual_body_md5, body_md5_from_query) {
                            debug!(
                                "body_md5 mismatch. Expected from query: {}, Calculated from body: {}",
                                body_md5_from_query, actual_body_md5
                            );
                            return Err(Error::Auth("body_md5 mismatch".to_string()));
                        }
                        // body_md5 is valid and already in params_for_signing_string
                    }
                    _ => {
                        // body_md5 is missing or empty in query, but POST body is non-empty
                        debug!(
                            "POST request has a non-empty body, but 'body_md5' is missing or empty in query parameters."
                        );
                        return Err(Error::Auth(
                            "body_md5 is required and must be non-empty in query parameters for POST requests with a non-empty body".to_string(),
                        ));
                    }
                }
            } else {
                // POST with an empty body
                if params_for_signing_string.contains_key("body_md5") {
                    debug!(
                        "POST request has an empty body, but 'body_md5' was found in query parameters."
                    );
                    return Err(Error::Auth(
                        "body_md5 must not be present in query parameters for POST requests with an empty body"
                            .to_string(),
                    ));
                }
                // No body_md5 to include for empty body POST
            }
        } else {
            // For GET requests (or other methods that are not POST)
            if params_for_signing_string.contains_key("body_md5") {
                debug!(
                    "{} request should not contain 'body_md5' in query parameters.",
                    uppercased_http_method
                );
                return Err(Error::Auth(format!(
                    "body_md5 must not be present in query parameters for {uppercased_http_method} requests"
                )));
            }
            // No body_md5 to include for GET
        }

        // --- Construct the string to sign ---
        // BTreeMap iterates in key-sorted order.
        // Keys must be lowercased for the signature string construction.
        let mut sorted_params_kv_pairs: Vec<String> = Vec::new();
        for (key, value) in &params_for_signing_string {
            sorted_params_kv_pairs.push(format!("{}={}", key.to_lowercase(), value));
        }
        let query_string_for_sig = sorted_params_kv_pairs.join("&");

        let string_to_sign =
            format!("{uppercased_http_method}\n{request_path}\n{query_string_for_sig}");
        debug!("String to sign: \n{}", string_to_sign); // Use \n for actual newline in log

        let token_signer = Token::new(app_config.key.clone(), app_config.secret.clone());
        let generated_signature = token_signer.sign(&string_to_sign);

        debug!("Generated signature: {}", generated_signature);
        debug!(
            "Received signature:  {}",
            auth_params_from_query_struct.auth_signature
        );

        if secure_compare(
            &generated_signature,
            &auth_params_from_query_struct.auth_signature,
        ) {
            Ok(true)
        } else {
            Err(Error::Auth("Invalid API signature".to_string()))
        }
    }

    pub fn sign_in_token_is_valid(
        &self,
        socket_id: &str,
        user_data: &str,
        expected_signature: &str,
        app_config: App,
    ) -> bool {
        let signature = self.sing_in_token_for_user_data(socket_id, user_data, app_config);
        secure_compare(&signature, expected_signature)
    }

    pub fn sing_in_token_for_user_data(
        &self,
        socket_id: &str,
        user_data: &str,
        app_config: App,
    ) -> String {
        let decoded_string = format!("{socket_id}::user::{user_data}");
        let signature = Token::new(app_config.key, app_config.secret);
        signature.sign(&decoded_string)
    }
}
