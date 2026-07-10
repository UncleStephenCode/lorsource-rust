-- Runtime migration compatibility for switching an existing Java/Liquibase LOR database
-- to the Rust implementation. 0001-0005 were enough for a clean Rust dev DB, but
-- some statements still assumed the old demo dump. This migration makes both
-- directions additive and safe: current Java schema -> Rust runtime, and old
-- demo dump -> current Java-compatible names.

CREATE EXTENSION IF NOT EXISTS hstore;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'markup_type') THEN
    CREATE TYPE markup_type AS ENUM ('PLAIN','BBCODE_TEX','BBCODE_ULB','MARKDOWN');
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'edit_event_type') THEN
    CREATE TYPE edit_event_type AS ENUM ('TOPIC','COMMENT');
  END IF;
END$$;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_type') THEN
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'WATCH';
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'DEL';
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'REF';
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'TAG';
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'REACTION';
    ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'WARNING';
  END IF;
END$$;

-- msgbase: current Java uses markup_type and dropped bbcode. Rust now reads markup
-- and derives bbcode compatibility in SELECTs.
ALTER TABLE msgbase ADD COLUMN IF NOT EXISTS markup text NOT NULL DEFAULT 'BBCODE_TEX';
UPDATE msgbase SET markup='BBCODE_TEX' WHERE markup IS NULL OR markup='LORCODE';

-- b_ips: current Java carries posting/captcha flags.
ALTER TABLE b_ips ADD COLUMN IF NOT EXISTS allow_posting boolean NOT NULL DEFAULT false;
ALTER TABLE b_ips ADD COLUMN IF NOT EXISTS captcha_required boolean NOT NULL DEFAULT false;

-- del/edit history additions from later Liquibase updates.
ALTER TABLE del_info ADD COLUMN IF NOT EXISTS bonus integer;
ALTER TABLE edit_info ADD COLUMN IF NOT EXISTS oldaddimages integer[];
ALTER TABLE edit_info ADD COLUMN IF NOT EXISTS oldminor boolean;
ALTER TABLE edit_info ADD COLUMN IF NOT EXISTS oldpoll text;

-- comments: current Java names are edit_date/editor_id; old Rust aliases were editdate/editor.
ALTER TABLE comments ADD COLUMN IF NOT EXISTS edit_date timestamp;
ALTER TABLE comments ADD COLUMN IF NOT EXISTS editor_id integer REFERENCES users(id);
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='comments' AND column_name='editdate') THEN
    EXECUTE 'UPDATE comments SET edit_date = COALESCE(edit_date, editdate) WHERE edit_date IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='comments' AND column_name='editor') THEN
    EXECUTE 'UPDATE comments SET editor_id = COALESCE(editor_id, editor) WHERE editor_id IS NULL';
  END IF;
END$$;

-- sections: keep Java column spelling in addition to the early Rust aliases.
ALTER TABLE sections ADD COLUMN IF NOT EXISTS imageallowed boolean NOT NULL DEFAULT false;
ALTER TABLE sections ADD COLUMN IF NOT EXISTS restrict_topics integer NOT NULL DEFAULT 0;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='sections' AND column_name='image_allowed') THEN
    EXECUTE 'UPDATE sections SET imageallowed = COALESCE(imageallowed, image_allowed)';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='sections' AND column_name='restrict_score') THEN
    EXECUTE 'UPDATE sections SET restrict_topics = COALESCE(restrict_topics, restrict_score)';
  END IF;
END$$;

-- images: current Java table has topic/extension/deleted/main; early Rust added userid/files.
ALTER TABLE images ADD COLUMN IF NOT EXISTS userid integer REFERENCES users(id);
ALTER TABLE images ADD COLUMN IF NOT EXISTS extension text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS icon text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS main boolean NOT NULL DEFAULT true;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='images' AND column_name='primary_image') THEN
    EXECUTE 'UPDATE images SET main = COALESCE(main, primary_image)';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='images' AND column_name='original') THEN
    EXECUTE $sql$UPDATE images SET extension = COALESCE(extension, lower(NULLIF(regexp_replace(original, '^.*\.([A-Za-z0-9]+)$', '\1'), original))) WHERE extension IS NULL$sql$;
  END IF;
END$$;
CREATE INDEX IF NOT EXISTS image_topic_idx ON images(topic);
CREATE UNIQUE INDEX IF NOT EXISTS images_uniq_idx ON images(id) WHERE NOT deleted AND main;

-- e-mail domain block current table: domain + block_until.
ALTER TABLE email_domains_block ADD COLUMN IF NOT EXISTS block_until timestamptz NOT NULL DEFAULT (now() + interval '100 years');

-- adv_counts current Java names are path/day/counter.
ALTER TABLE adv_counts ADD COLUMN IF NOT EXISTS path text;
ALTER TABLE adv_counts ADD COLUMN IF NOT EXISTS day date;
ALTER TABLE adv_counts ADD COLUMN IF NOT EXISTS counter bigint NOT NULL DEFAULT 0;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='adv_counts' AND column_name='adv') THEN
    EXECUTE 'UPDATE adv_counts SET path = COALESCE(path, adv) WHERE path IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='adv_counts' AND column_name='event_date') THEN
    EXECUTE 'UPDATE adv_counts SET day = COALESCE(day, event_date) WHERE day IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='adv_counts' AND column_name='views') THEN
    EXECUTE 'UPDATE adv_counts SET counter = GREATEST(counter, views)';
  END IF;
END$$;
CREATE UNIQUE INDEX IF NOT EXISTS adv_counts_unique ON adv_counts(path, day);

-- tag synonyms current Java names are tagid/value.
ALTER TABLE tags_synonyms ADD COLUMN IF NOT EXISTS tagid integer REFERENCES tags_values(id);
ALTER TABLE tags_synonyms ADD COLUMN IF NOT EXISTS value text;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='tags_synonyms' AND column_name='tag_id') THEN
    EXECUTE 'UPDATE tags_synonyms SET tagid = COALESCE(tagid, tag_id) WHERE tagid IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='tags_synonyms' AND column_name='synonym') THEN
    EXECUTE 'UPDATE tags_synonyms SET value = COALESCE(value, synonym) WHERE value IS NULL';
  END IF;
END$$;
CREATE UNIQUE INDEX IF NOT EXISTS tags_synonyms_value_idx ON tags_synonyms(value);

-- telegram_posts current Java uses topic_id.
ALTER TABLE telegram_posts ADD COLUMN IF NOT EXISTS topic_id integer REFERENCES topics(id);
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='telegram_posts' AND column_name='topic') THEN
    EXECUTE 'UPDATE telegram_posts SET topic_id = COALESCE(topic_id, topic) WHERE topic_id IS NULL';
  END IF;
END$$;

-- topics/users late fields.
ALTER TABLE topics ADD COLUMN IF NOT EXISTS allow_anonymous boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS token_generation integer NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN IF NOT EXISTS frozen_by integer REFERENCES users(id);
ALTER TABLE users ADD COLUMN IF NOT EXISTS freezing_reason text;
ALTER TABLE users ADD COLUMN IF NOT EXISTS userinfo_markup text NOT NULL DEFAULT 'MARKDOWN';

-- user_events current Java includes origin_user.
ALTER TABLE user_events ADD COLUMN IF NOT EXISTS origin_user integer REFERENCES users(id);
UPDATE user_events SET origin_user = COALESCE(origin_user, userid) WHERE origin_user IS NULL;

-- user_remarks current Java names. Keep old aliases if they exist, but all Rust
-- runtime SQL now uses user_id/ref_user_id/remark_text.
CREATE SEQUENCE IF NOT EXISTS user_remarks_id_seq;
ALTER TABLE user_remarks ADD COLUMN IF NOT EXISTS id integer;
ALTER TABLE user_remarks ALTER COLUMN id SET DEFAULT nextval('user_remarks_id_seq');
ALTER TABLE user_remarks ADD COLUMN IF NOT EXISTS user_id integer REFERENCES users(id);
ALTER TABLE user_remarks ADD COLUMN IF NOT EXISTS ref_user_id integer REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE user_remarks ADD COLUMN IF NOT EXISTS remark_text text;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='user_remarks' AND column_name='userid') THEN
    EXECUTE 'UPDATE user_remarks SET user_id = COALESCE(user_id, userid) WHERE user_id IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='user_remarks' AND column_name='who') THEN
    EXECUTE 'UPDATE user_remarks SET ref_user_id = COALESCE(ref_user_id, who) WHERE ref_user_id IS NULL';
  END IF;
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='user_remarks' AND column_name='remark') THEN
    EXECUTE 'UPDATE user_remarks SET remark_text = COALESCE(remark_text, remark) WHERE remark_text IS NULL';
  END IF;
END$$;
UPDATE user_remarks SET id = nextval('user_remarks_id_seq') WHERE id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS user_remarks_userid_refuser ON user_remarks(user_id, ref_user_id);
CREATE INDEX IF NOT EXISTS user_remarks_userid_idx ON user_remarks(user_id);
CREATE INDEX IF NOT EXISTS refuser_remarks_tagid_idx ON user_remarks(ref_user_id);

-- user_tags current Java names. Rust runtime SQL now uses user_id/tag_id/is_favorite.
ALTER TABLE user_tags ADD COLUMN IF NOT EXISTS user_id integer REFERENCES users(id);
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='user_tags' AND column_name='userid') THEN
    EXECUTE 'UPDATE user_tags SET user_id = COALESCE(user_id, userid) WHERE user_id IS NULL';
  END IF;
END$$;
CREATE UNIQUE INDEX IF NOT EXISTS user_tags_uniq_idx ON user_tags(user_id, tag_id, is_favorite);
CREATE INDEX IF NOT EXISTS user_tags_userid_idx ON user_tags(user_id);
CREATE INDEX IF NOT EXISTS user_tags_tagid_idx ON user_tags(tag_id);
