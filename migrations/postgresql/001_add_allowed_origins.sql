-- Add allowed_origins column to PostgreSQL applications table
-- This migration adds support for per-app origin validation

-- Add the column if it doesn't exist
ALTER TABLE applications ADD COLUMN IF NOT EXISTS allowed_origins JSONB;

-- Create an index for better query performance (optional)
CREATE INDEX IF NOT EXISTS idx_applications_allowed_origins 
ON applications USING gin(allowed_origins);

-- Example of updating an existing app with allowed origins
-- UPDATE applications 
-- SET allowed_origins = '["https://app.example.com", "*.staging.example.com", "http://localhost:3000"]'::jsonb
-- WHERE id = 'your-app-id';