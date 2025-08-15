-- =============================================================================
-- Sockudo Test Database Initialization (Part 2)
-- =============================================================================
-- This script runs after 01-init-sql.sql and adds test-specific configurations

-- Create sockudo test user with access from any host and localhost
CREATE USER IF NOT EXISTS 'sockudo'@'%' IDENTIFIED BY 'sockudo123';
CREATE USER IF NOT EXISTS 'sockudo'@'localhost' IDENTIFIED BY 'sockudo123';

-- Grant all necessary permissions to sockudo user for testing
GRANT ALL PRIVILEGES ON sockudo.* TO 'sockudo'@'%';
GRANT ALL PRIVILEGES ON sockudo.* TO 'sockudo'@'localhost';

FLUSH PRIVILEGES;