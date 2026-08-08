-- The real Java schema has no users.style column - it was migrated into
-- user_settings.settings ('style' hstore key) years ago (see the comment in
-- 0004_current_java_schema_compat.sql, which already copied users.style into
-- user_settings for any pre-existing rows). Drop the column so the schema
-- matches the real production database exactly.
ALTER TABLE users DROP COLUMN IF EXISTS style;
