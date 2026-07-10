-- Compatibility tables for the original lorsource schema and later Liquibase updates.
-- This migration is intentionally additive: existing MVP tables from 0001 are kept,
-- and missing legacy tables are created so service-by-service porting can be done
-- without changing table names again.

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_type') THEN
    CREATE TYPE event_type AS ENUM ('REPLY','COMMENT','MENTION','MEMORIES','REACTION','WARNING','OTHER');
  END IF;
END$$;

CREATE TABLE IF NOT EXISTS b_ips (
    ip inet PRIMARY KEY,
    mod_id integer NOT NULL REFERENCES users(id),
    date timestamptz NOT NULL DEFAULT now(),
    reason varchar(255),
    ban_date timestamp
);

CREATE TABLE IF NOT EXISTS ban_info (
    userid integer PRIMARY KEY REFERENCES users(id),
    bandate timestamp NOT NULL DEFAULT now(),
    reason text NOT NULL,
    ban_by integer NOT NULL REFERENCES users(id)
);

CREATE TABLE IF NOT EXISTS del_info (
    msgid integer PRIMARY KEY,
    delby integer NOT NULL REFERENCES users(id),
    reason text,
    deldate timestamp DEFAULT now()
);

CREATE TABLE IF NOT EXISTS edit_info (
    id serial PRIMARY KEY,
    msgid integer NOT NULL,
    editor integer NOT NULL REFERENCES users(id),
    oldmessage text,
    editdate timestamp NOT NULL DEFAULT now(),
    oldtitle text,
    oldtags text,
    oldlinktext text,
    oldurl text,
    oldimage integer,
    object_type text NOT NULL DEFAULT 'TOPIC',
    minor boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS ignore_list (
    userid integer NOT NULL REFERENCES users(id),
    ignored integer NOT NULL REFERENCES users(id),
    PRIMARY KEY(userid, ignored)
);

CREATE TABLE IF NOT EXISTS memories (
    id serial PRIMARY KEY,
    userid integer NOT NULL REFERENCES users(id),
    topic integer NOT NULL REFERENCES topics(id),
    add_date timestamp NOT NULL DEFAULT now(),
    watch boolean NOT NULL DEFAULT false,
    notify boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS monthly_stats (
    section integer REFERENCES sections(id),
    year integer NOT NULL,
    month integer NOT NULL,
    c integer NOT NULL DEFAULT 0,
    groupid integer REFERENCES groups(id)
);

CREATE INDEX IF NOT EXISTS monthly_stats_section_idx ON monthly_stats(section, year, month);
CREATE INDEX IF NOT EXISTS monthly_stats_group_idx ON monthly_stats(groupid, year, month);

CREATE TABLE IF NOT EXISTS user_agents (
    id serial PRIMARY KEY,
    name varchar(512) DEFAULT ''
);

CREATE TABLE IF NOT EXISTS votenames (
    id serial PRIMARY KEY,
    topic integer NOT NULL DEFAULT 0 REFERENCES topics(id),
    multiselect boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS votes (
    id serial PRIMARY KEY,
    vote integer NOT NULL REFERENCES votenames(id),
    label text NOT NULL,
    votes integer NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS vote_users (
    vote integer NOT NULL REFERENCES votes(id),
    userid integer NOT NULL REFERENCES users(id),
    PRIMARY KEY(vote, userid)
);

CREATE TABLE IF NOT EXISTS user_tags (
    userid integer NOT NULL REFERENCES users(id),
    tag_id integer NOT NULL REFERENCES tags_values(id),
    is_favorite boolean NOT NULL DEFAULT true,
    PRIMARY KEY(userid, tag_id, is_favorite)
);

CREATE TABLE IF NOT EXISTS user_remarks (
    userid integer NOT NULL REFERENCES users(id),
    who integer NOT NULL REFERENCES users(id),
    remark text NOT NULL,
    PRIMARY KEY(userid, who)
);

CREATE TABLE IF NOT EXISTS persistent_logins (
    username varchar(64) NOT NULL,
    series varchar(64) PRIMARY KEY,
    token varchar(64) NOT NULL,
    last_used timestamp NOT NULL
);

CREATE TABLE IF NOT EXISTS topic_users_notified (
    topic integer NOT NULL REFERENCES topics(id),
    userid integer NOT NULL REFERENCES users(id),
    PRIMARY KEY(topic, userid)
);

CREATE TABLE IF NOT EXISTS images (
    id serial PRIMARY KEY,
    userid integer NOT NULL REFERENCES users(id),
    topic integer REFERENCES topics(id),
    original text,
    medium text,
    thumbnail text,
    deleted boolean NOT NULL DEFAULT false,
    postdate timestamptz NOT NULL DEFAULT now(),
    width integer,
    height integer,
    original_name text,
    primary_image boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS telegram_posts (
    topic integer PRIMARY KEY REFERENCES topics(id),
    telegram_id bigint NOT NULL,
    postdate timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS user_invites (
    id serial PRIMARY KEY,
    owner integer NOT NULL REFERENCES users(id),
    invite_code uuid NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    used_by integer REFERENCES users(id),
    used_at timestamptz
);

CREATE TABLE IF NOT EXISTS tags_synonyms (
    id serial PRIMARY KEY,
    tag_id integer NOT NULL REFERENCES tags_values(id),
    synonym text NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS reactions_log (
    id serial PRIMARY KEY,
    userid integer NOT NULL REFERENCES users(id),
    msgid integer NOT NULL,
    reaction text NOT NULL,
    set_value boolean NOT NULL DEFAULT true,
    action_date timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS adv_counts (
    id serial PRIMARY KEY,
    adv text NOT NULL,
    event_date date NOT NULL DEFAULT CURRENT_DATE,
    views bigint NOT NULL DEFAULT 0,
    clicks bigint NOT NULL DEFAULT 0,
    UNIQUE(adv, event_date)
);

CREATE TABLE IF NOT EXISTS message_warnings (
    id serial PRIMARY KEY,
    userid integer NOT NULL REFERENCES users(id),
    moderator integer NOT NULL REFERENCES users(id),
    topic_id integer REFERENCES topics(id),
    comment_id integer REFERENCES comments(id),
    reason text NOT NULL,
    postdate timestamptz NOT NULL DEFAULT now(),
    resolved boolean NOT NULL DEFAULT false,
    resolved_at timestamptz
);

CREATE TABLE IF NOT EXISTS email_domains_block (
    domain text PRIMARY KEY,
    reason text,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Add columns introduced by later Liquibase updates where the MVP schema kept only
-- fields that were already used by the first Rust handlers.
ALTER TABLE users ADD COLUMN IF NOT EXISTS force_unlogin integer NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN IF NOT EXISTS frozen_until timestamptz;
ALTER TABLE users ADD COLUMN IF NOT EXISTS settings jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE users ADD COLUMN IF NOT EXISTS userinfo_markup text;

ALTER TABLE topics ADD COLUMN IF NOT EXISTS draft boolean NOT NULL DEFAULT false;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS no_comments boolean NOT NULL DEFAULT false;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS reactions jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS image integer REFERENCES images(id);
ALTER TABLE topics ADD COLUMN IF NOT EXISTS warning_counter integer NOT NULL DEFAULT 0;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS score_loss integer NOT NULL DEFAULT 0;

ALTER TABLE comments ADD COLUMN IF NOT EXISTS editdate timestamp;
ALTER TABLE comments ADD COLUMN IF NOT EXISTS editor integer REFERENCES users(id);
ALTER TABLE comments ADD COLUMN IF NOT EXISTS edit_count integer NOT NULL DEFAULT 0;
ALTER TABLE comments ADD COLUMN IF NOT EXISTS reactions jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE sections ADD COLUMN IF NOT EXISTS scroll_mode text NOT NULL DEFAULT 'NORMAL';
ALTER TABLE sections ADD COLUMN IF NOT EXISTS restrict_score integer NOT NULL DEFAULT 0;
ALTER TABLE sections ADD COLUMN IF NOT EXISTS image_allowed boolean NOT NULL DEFAULT false;

ALTER TABLE msgbase ADD COLUMN IF NOT EXISTS markup text NOT NULL DEFAULT 'LORCODE';

ALTER TABLE user_events ADD COLUMN IF NOT EXISTS type event_type NOT NULL DEFAULT 'OTHER';
ALTER TABLE user_events ADD COLUMN IF NOT EXISTS private boolean NOT NULL DEFAULT false;
ALTER TABLE user_events ADD COLUMN IF NOT EXISTS message_id integer;
ALTER TABLE user_events ADD COLUMN IF NOT EXISTS comment_id integer;
ALTER TABLE user_events ADD COLUMN IF NOT EXISTS unread boolean NOT NULL DEFAULT true;

CREATE INDEX IF NOT EXISTS memories_userid_idx ON memories(userid);
CREATE INDEX IF NOT EXISTS memories_topic_idx ON memories(topic);
CREATE INDEX IF NOT EXISTS edit_info_msgid_idx ON edit_info(msgid);
CREATE INDEX IF NOT EXISTS images_topic_idx ON images(topic);
CREATE INDEX IF NOT EXISTS reactions_log_msgid_idx ON reactions_log(msgid);
CREATE INDEX IF NOT EXISTS message_warnings_userid_idx ON message_warnings(userid);
CREATE INDEX IF NOT EXISTS user_events_userid_unread_idx ON user_events(userid, unread);
