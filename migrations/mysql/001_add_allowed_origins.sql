-- Add allowed_origins column to MySQL applications table
-- This migration adds support for per-app origin validation

-- Check if the column doesn't exist and add it
SET @exists = (SELECT COUNT(*) 
               FROM INFORMATION_SCHEMA.COLUMNS 
               WHERE TABLE_SCHEMA = DATABASE() 
               AND TABLE_NAME = 'applications' 
               AND COLUMN_NAME = 'allowed_origins');

-- Add the column if it doesn't exist
SET @sql = IF(@exists = 0, 'ALTER TABLE applications ADD COLUMN allowed_origins JSON NULL', 'SELECT "Column already exists" AS message');
PREPARE stmt FROM @sql;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

-- Example of updating an existing app with allowed origins
-- UPDATE applications 
-- SET allowed_origins = JSON_ARRAY('https://app.example.com', '*.staging.example.com', 'http://localhost:3000')
-- WHERE id = 'your-app-id';