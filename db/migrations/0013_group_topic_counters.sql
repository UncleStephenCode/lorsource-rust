-- Wires up the real comins()/topins()/msgdel()/msgundel() triggers that
-- Java relies on to keep topics.stat1/stat3 and groups.stat3 in sync, and
-- to auto-subscribe a topic's author to it via `memories` (skipping the
-- anonymous user, id=2) - see sql/updates/2015-03-03-remove-topics-stat2.xml
-- and 2026-05-14-anon-no-memories.xml upstream for the exact final bodies.
-- Rust never wrote these counters at all before this migration, so groups'
-- activity counters were permanently stuck at their seed values, and new
-- topics never auto-subscribed their author to notifications.
--
-- comments.topic_deleted was dropped by Java in
-- sql/updates/2014-11-15-remove-topic-deleted-column.xml; Rust's own
-- earlier migration invented it from scratch (CREATE TABLE IF NOT EXISTS
-- silently no-ops against a real Java DB, which never has this column), so
-- any query referencing it would fail against a real migrated database.
-- Drop it here and stop relying on it (topic-level visibility is handled by
-- checking topics.deleted directly, not a redundant per-comment flag).
ALTER TABLE comments DROP COLUMN IF EXISTS topic_deleted;

CREATE OR REPLACE FUNCTION comins() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
        cgroup int;
BEGIN
        SELECT groupid INTO cgroup FROM topics WHERE topics.id = NEW.topic;
        UPDATE topics SET stat1=stat1+1,stat3=stat3+1,lastmod=CURRENT_TIMESTAMP WHERE topics.id = NEW.topic;
        UPDATE groups SET stat3=stat3+1 WHERE id = cgroup;
        RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION topins() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE groups SET stat3=stat3+1 WHERE groups.id = NEW.groupid;
    UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id = NEW.id;
    IF NEW.userid != 2 THEN
      INSERT INTO memories (userid, topic) VALUES (NEW.userid, NEW.id);
    END IF;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION msgdel() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id = NEW.msgid;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION msgundel() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id = OLD.msgid;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS comins_t ON comments;
CREATE TRIGGER comins_t AFTER INSERT ON comments FOR EACH ROW EXECUTE PROCEDURE comins();

DROP TRIGGER IF EXISTS topins_t ON topics;
CREATE TRIGGER topins_t AFTER INSERT ON topics FOR EACH ROW EXECUTE PROCEDURE topins();

DROP TRIGGER IF EXISTS msgdel_t ON del_info;
CREATE TRIGGER msgdel_t AFTER INSERT ON del_info FOR EACH ROW EXECUTE PROCEDURE msgdel();

DROP TRIGGER IF EXISTS msgundel_t ON del_info;
CREATE TRIGGER msgundel_t AFTER DELETE ON del_info FOR EACH ROW EXECUTE PROCEDURE msgundel();
