-- legacy::memories (POST /memories.jsp) upserts via
-- ON CONFLICT(userid,topic), but no unique constraint ever backed that -
-- every call failed with "no unique or exclusion constraint matching the
-- ON CONFLICT specification". Java's MemoriesDao does a plain INSERT and
-- relies on the caller not double-adding, so this constraint is Rust-only,
-- added here to make the existing ON CONFLICT upsert actually work.
CREATE UNIQUE INDEX IF NOT EXISTS memories_userid_topic_idx ON memories(userid, topic);
