-- Corrections found by re-checking the Rust port against the uploaded current Java/Scala source.
-- The earlier compatibility layer covered endpoint/table names but several current Liquibase
-- structures still used draft column names. Keep old compatibility columns where they exist,
-- but add/backfill the exact columns used by the Java DAOs.

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'warning_type') THEN
    CREATE TYPE warning_type AS ENUM ('rule', 'tag', 'spelling', 'group');
  END IF;
END$$;

-- Current Java warning model: message_warnings(topic, comment, author, message, warning_type, closed_by, closed_when).
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS topic integer REFERENCES topics(id);
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS comment integer REFERENCES comments(id);
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS author integer REFERENCES users(id);
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS message text;
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS warning_type warning_type NOT NULL DEFAULT 'rule';
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS closed_by integer REFERENCES users(id);
ALTER TABLE message_warnings ADD COLUMN IF NOT EXISTS closed_when timestamptz;

UPDATE message_warnings
SET topic = COALESCE(topic, topic_id),
    comment = COALESCE(comment, comment_id),
    author = COALESCE(author, moderator, userid),
    message = COALESCE(message, reason)
WHERE topic IS NULL OR comment IS NULL OR author IS NULL OR message IS NULL;

CREATE INDEX IF NOT EXISTS message_warnings_topic_idx ON message_warnings(topic);
CREATE INDEX IF NOT EXISTS message_warnings_comment_idx ON message_warnings(comment);
CREATE INDEX IF NOT EXISTS message_warnings_author_idx ON message_warnings(author);
CREATE INDEX IF NOT EXISTS message_warnings_postdate_idx ON message_warnings(postdate);
CREATE INDEX IF NOT EXISTS message_warnings_closed_by_idx ON message_warnings(closed_by);

ALTER TABLE user_events ADD COLUMN IF NOT EXISTS warning_id integer REFERENCES message_warnings(id);
CREATE INDEX IF NOT EXISTS user_events_warning_id_idx ON user_events(warning_id);

-- Current Java topic warning counter is open_warnings, not warning_counter.
ALTER TABLE topics ADD COLUMN IF NOT EXISTS open_warnings integer NOT NULL DEFAULT 0;
UPDATE topics
SET open_warnings = warning_counter
WHERE open_warnings = 0 AND warning_counter IS NOT NULL;

-- Current Java reaction log model: origin_user, topic_id, comment_id, set_date, reaction.
ALTER TABLE reactions_log ADD COLUMN IF NOT EXISTS origin_user integer REFERENCES users(id);
ALTER TABLE reactions_log ADD COLUMN IF NOT EXISTS topic_id integer REFERENCES topics(id);
ALTER TABLE reactions_log ADD COLUMN IF NOT EXISTS comment_id integer REFERENCES comments(id);
ALTER TABLE reactions_log ADD COLUMN IF NOT EXISTS set_date timestamptz NOT NULL DEFAULT now();
ALTER TABLE reactions_log ADD COLUMN IF NOT EXISTS reaction text;

UPDATE reactions_log rl
SET origin_user = COALESCE(origin_user, userid),
    topic_id = COALESCE(topic_id, CASE WHEN c.topic IS NOT NULL THEN c.topic ELSE rl.msgid END),
    comment_id = COALESCE(comment_id, CASE WHEN c.id IS NOT NULL THEN rl.msgid ELSE NULL END),
    set_date = COALESCE(set_date, action_date, now())
FROM comments c
WHERE rl.msgid = c.id
  AND (rl.origin_user IS NULL OR rl.topic_id IS NULL OR rl.comment_id IS NULL);

UPDATE reactions_log
SET origin_user = COALESCE(origin_user, userid),
    topic_id = COALESCE(topic_id, msgid),
    set_date = COALESCE(set_date, action_date, now())
WHERE origin_user IS NULL OR topic_id IS NULL;

DROP INDEX IF EXISTS reactions_log_upsert_idx;
CREATE UNIQUE INDEX IF NOT EXISTS reactions_log_upsert_idx
  ON reactions_log(topic_id, comment_id, origin_user) NULLS NOT DISTINCT;
CREATE INDEX IF NOT EXISTS reactions_log_origin_user_idx ON reactions_log(origin_user);
CREATE INDEX IF NOT EXISTS reactions_log_topic_idx ON reactions_log(topic_id);
CREATE INDEX IF NOT EXISTS reactions_log_comment_idx ON reactions_log(comment_id);
CREATE INDEX IF NOT EXISTS reactions_log_user_date_idx ON reactions_log(origin_user, set_date);

-- Current Java invite model: invite_code text primary key, owner, issue_date, invited_user, email, valid_until.
ALTER TABLE user_invites ADD COLUMN IF NOT EXISTS invite_code text;
ALTER TABLE user_invites ALTER COLUMN invite_code TYPE text USING invite_code::text;
ALTER TABLE user_invites ADD COLUMN IF NOT EXISTS issue_date timestamptz NOT NULL DEFAULT now();
ALTER TABLE user_invites ADD COLUMN IF NOT EXISTS invited_user integer REFERENCES users(id);
ALTER TABLE user_invites ADD COLUMN IF NOT EXISTS email text;
ALTER TABLE user_invites ADD COLUMN IF NOT EXISTS valid_until timestamptz;

UPDATE user_invites
SET issue_date = COALESCE(issue_date, created_at),
    invited_user = COALESCE(invited_user, used_by),
    email = COALESCE(email, ''),
    valid_until = COALESCE(valid_until, used_at + interval '1 day', created_at + interval '7 days')
WHERE valid_until IS NULL OR email IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS user_invites_invite_code_idx ON user_invites(invite_code);
CREATE INDEX IF NOT EXISTS user_invites_owner_idx ON user_invites(owner);
CREATE INDEX IF NOT EXISTS user_invites_invited_user_idx ON user_invites(invited_user);
