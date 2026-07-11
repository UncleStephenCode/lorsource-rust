-- 0011_memories_unique.sql made memories unique on (userid,topic), which
-- makes "watch" and "favorite" mutually exclusive per topic - toggling one
-- silently overwrites the other. Java's real constraint (see
-- sql/updates/2012-05-11-memories-type.xml, "memories_un") is
-- (userid,topic,watch): a favorite row (watch=false) and a watch row
-- (watch=true) are independent rows that can coexist for the same topic.
DROP INDEX IF EXISTS memories_userid_topic_idx;
CREATE UNIQUE INDEX IF NOT EXISTS memories_un ON memories(userid, topic, watch);
