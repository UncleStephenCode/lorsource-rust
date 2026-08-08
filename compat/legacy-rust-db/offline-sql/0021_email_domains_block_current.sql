-- Align the old Rust-only email block scaffold with the current Java/Liquibase
-- table while remaining safe on an already migrated Java database.
ALTER TABLE email_domains_block ADD COLUMN IF NOT EXISTS block_until timestamptz;
ALTER TABLE email_domains_block ADD COLUMN IF NOT EXISTS auto boolean NOT NULL DEFAULT false;
ALTER TABLE email_domains_block ADD COLUMN IF NOT EXISTS moderator_id integer;
ALTER TABLE email_domains_block ADD COLUMN IF NOT EXISTS blocked_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP;

-- Only old Rust development rows can lack an expiry. Preserve their blocking
-- intent using the same three-year manual period used by the current UI.
UPDATE email_domains_block
SET block_until = CURRENT_TIMESTAMP + interval '3 years'
WHERE block_until IS NULL;

ALTER TABLE email_domains_block ALTER COLUMN block_until SET NOT NULL;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'email_domains_block_moderator_fkey'
  ) THEN
    ALTER TABLE email_domains_block
      ADD CONSTRAINT email_domains_block_moderator_fkey
      FOREIGN KEY (moderator_id) REFERENCES users(id) ON DELETE SET NULL;
  END IF;
END $$;
