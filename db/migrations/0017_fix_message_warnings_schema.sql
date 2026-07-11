-- message_warnings was created with an invented legacy-style schema
-- (userid/moderator/reason/topic_id/comment_id/resolved/resolved_at, most
-- NOT NULL with no default) alongside the real columns the app code
-- actually uses (topic/comment/postdate/author/message/warning_type/
-- closed_by/closed_when, matching Java's 2024-10-26-warnings.xml exactly).
-- Since the app never populates userid/moderator/reason, every single
-- INSERT INTO message_warnings has been failing a NOT NULL constraint
-- violation - no warning has ever actually been recorded, and
-- post_warning's `let _ = warning_id` was masking a 500 that occurred
-- before that line was ever reached.
ALTER TABLE message_warnings DROP COLUMN IF EXISTS userid;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS moderator;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS topic_id;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS comment_id;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS reason;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS resolved;
ALTER TABLE message_warnings DROP COLUMN IF EXISTS resolved_at;

ALTER TABLE message_warnings ALTER COLUMN topic SET NOT NULL;
ALTER TABLE message_warnings ALTER COLUMN author SET NOT NULL;
ALTER TABLE message_warnings ALTER COLUMN message SET NOT NULL;
