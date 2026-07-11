-- Wires up the previously-stubbed notifications system (user_events).
--
-- event_type previously used Rust-only labels (COMMENT/MENTION/MEMORIES/OTHER)
-- that don't exist on the real Java enum (UserEventFilterEnum.dbType: REPLY,
-- WATCH, DEL, REF, TAG, REACTION, WARNING - see sql/updates/2012-03-30-add-
-- value-to-event_type-9.1.xml, 2022-11-30-reaction-events.xml, warnings
-- changesets). Nothing in Rust read/wrote user_events before this pass, so
-- it's safe to extend the enum now that the feature goes live.
ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'WATCH';
ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'DEL';
ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'REF';
ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'TAG';

ALTER TABLE user_events ADD COLUMN IF NOT EXISTS origin_user integer REFERENCES users(id);
CREATE INDEX IF NOT EXISTS user_events_origin_idx ON user_events(origin_user);
CREATE INDEX IF NOT EXISTS user_events_userid_id_idx ON user_events(userid, id DESC);
