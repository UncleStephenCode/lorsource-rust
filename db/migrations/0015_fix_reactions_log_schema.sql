-- reactions_log was created with an invented legacy-style schema
-- (userid/msgid/set_value/action_date, all NOT NULL with no default)
-- alongside the real columns the app code actually uses
-- (origin_user/topic_id/comment_id/set_date/reaction, matching Java's
-- 2023-09-23-reactions-log.xml exactly). Since the app never populates
-- userid/msgid, every single INSERT INTO reactions_log has been failing
-- a NOT NULL constraint violation - no reaction has ever actually been
-- recorded. Drop the invented columns and align nullability with Java.
ALTER TABLE reactions_log DROP COLUMN IF EXISTS userid;
ALTER TABLE reactions_log DROP COLUMN IF EXISTS msgid;
ALTER TABLE reactions_log DROP COLUMN IF EXISTS set_value;
ALTER TABLE reactions_log DROP COLUMN IF EXISTS action_date;

ALTER TABLE reactions_log ALTER COLUMN origin_user SET NOT NULL;
ALTER TABLE reactions_log ALTER COLUMN topic_id SET NOT NULL;
ALTER TABLE reactions_log ALTER COLUMN set_date SET NOT NULL;
