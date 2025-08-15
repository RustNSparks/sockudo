-- =============================================================================
-- Sockudo Docker Database Initialization (without stored procedures)
-- =============================================================================

-- Create applications table
CREATE TABLE IF NOT EXISTS applications (
                                            id VARCHAR(255) NOT NULL PRIMARY KEY,
    `key` VARCHAR(255) NOT NULL UNIQUE,
    secret VARCHAR(255) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    -- Connection limits
    max_connections INT UNSIGNED DEFAULT 1000,
    max_client_events_per_second INT UNSIGNED DEFAULT 100,
    max_read_requests_per_second INT UNSIGNED DEFAULT 100,
    max_backend_events_per_second INT UNSIGNED DEFAULT 100,

    -- Channel settings
    max_channel_name_length INT UNSIGNED DEFAULT 200,
    max_event_name_length INT UNSIGNED DEFAULT 200,
    max_event_payload_in_kb INT UNSIGNED DEFAULT 100,
    max_event_channels_at_once INT UNSIGNED DEFAULT 100,
    max_event_batch_size INT UNSIGNED DEFAULT 10,

    -- Presence settings
    max_presence_members_per_channel INT UNSIGNED DEFAULT 100,
    max_presence_member_size_in_kb INT UNSIGNED DEFAULT 2,

    -- Feature flags
    enable_client_messages BOOLEAN DEFAULT FALSE,
    enable_user_authentication BOOLEAN DEFAULT FALSE,
    enable_watchlist_events BOOLEAN DEFAULT FALSE,

    -- Webhooks (JSON column for webhook configurations)
    webhooks JSON DEFAULT NULL,

    -- Timestamps
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    -- Indexes
    INDEX idx_key (`key`),
    INDEX idx_enabled (enabled),
    INDEX idx_created_at (created_at)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Create application_stats table for tracking usage
CREATE TABLE IF NOT EXISTS application_stats (
                                                 id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
                                                 app_id VARCHAR(255) NOT NULL,
    metric_name VARCHAR(100) NOT NULL,
    metric_value BIGINT UNSIGNED NOT NULL,
    recorded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (app_id) REFERENCES applications(id) ON DELETE CASCADE,
    INDEX idx_app_metric (app_id, metric_name),
    INDEX idx_recorded_at (recorded_at)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Create sessions table for tracking active connections
CREATE TABLE IF NOT EXISTS sessions (
                                        socket_id VARCHAR(255) NOT NULL PRIMARY KEY,
    app_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) DEFAULT NULL,
    connected_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    FOREIGN KEY (app_id) REFERENCES applications(id) ON DELETE CASCADE,
    INDEX idx_app_id (app_id),
    INDEX idx_user_id (user_id),
    INDEX idx_connected_at (connected_at),
    INDEX idx_last_activity (last_activity)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Create webhooks_log table for webhook delivery tracking
CREATE TABLE IF NOT EXISTS webhooks_log (
                                            id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
                                            app_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    webhook_url VARCHAR(500) DEFAULT NULL,
    lambda_function VARCHAR(255) DEFAULT NULL,
    payload JSON NOT NULL,
    status_code INT DEFAULT NULL,
    response_body TEXT DEFAULT NULL,
    attempt_count INT UNSIGNED DEFAULT 1,
    delivered_at TIMESTAMP DEFAULT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (app_id) REFERENCES applications(id) ON DELETE CASCADE,
    INDEX idx_app_event (app_id, event_type),
    INDEX idx_status (status_code),
    INDEX idx_created_at (created_at)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Create views for monitoring
CREATE VIEW active_applications AS
SELECT
    a.id,
    a.`key`,
    a.enabled,
    COUNT(s.socket_id) as active_connections,
    MAX(s.last_activity) as last_connection_activity
FROM applications a
         LEFT JOIN sessions s ON a.id = s.app_id
WHERE a.enabled = TRUE
GROUP BY a.id, a.`key`, a.enabled;

CREATE VIEW webhook_delivery_stats AS
SELECT
    app_id,
    event_type,
    COUNT(*) as total_attempts,
    SUM(CASE WHEN status_code BETWEEN 200 AND 299 THEN 1 ELSE 0 END) as successful_deliveries,
    AVG(attempt_count) as avg_attempts_per_webhook,
    MAX(created_at) as last_webhook_at
FROM webhooks_log
WHERE created_at > DATE_SUB(NOW(), INTERVAL 24 HOUR)
GROUP BY app_id, event_type;

-- Create sockudo user with access from any host
CREATE USER IF NOT EXISTS 'sockudo'@'%' IDENTIFIED BY 'sockudo123';

-- Grant permissions to sockudo user
GRANT SELECT, INSERT, UPDATE, DELETE ON sockudo.* TO 'sockudo'@'%';
GRANT SELECT ON sockudo.active_applications TO 'sockudo'@'%';
GRANT SELECT ON sockudo.webhook_delivery_stats TO 'sockudo'@'%';

FLUSH PRIVILEGES;