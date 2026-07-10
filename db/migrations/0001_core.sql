-- Minimal PostgreSQL schema compatible with the Rust port and close to the historic LOR tables.
CREATE SEQUENCE IF NOT EXISTS s_uid START WITH 10;
CREATE SEQUENCE IF NOT EXISTS s_msgid START WITH 100;

CREATE TABLE IF NOT EXISTS sections (
    id integer PRIMARY KEY,
    name varchar(255) NOT NULL,
    moderate boolean NOT NULL DEFAULT false,
    imagepost boolean NOT NULL DEFAULT false,
    preformat boolean NOT NULL DEFAULT false,
    linktext varchar(255),
    havelink boolean NOT NULL DEFAULT false,
    expire interval NOT NULL DEFAULT interval '365 days',
    vote boolean DEFAULT false,
    add_info text
);

CREATE TABLE IF NOT EXISTS groups (
    id integer PRIMARY KEY,
    title varchar(255) NOT NULL,
    image varchar(255),
    section integer NOT NULL REFERENCES sections(id),
    stat1 integer NOT NULL DEFAULT 0,
    stat2 integer NOT NULL DEFAULT 0,
    stat3 integer NOT NULL DEFAULT 0,
    stat4 integer NOT NULL DEFAULT 0,
    restrict_topics integer,
    info text,
    restrict_comments integer NOT NULL DEFAULT -9999,
    longinfo text,
    resolvable boolean NOT NULL DEFAULT false,
    urlname text NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS users (
    id integer PRIMARY KEY DEFAULT nextval('s_uid'),
    name varchar(255),
    nick varchar(80) NOT NULL UNIQUE,
    passwd varchar(255),
    url varchar(255),
    email varchar(255),
    canmod boolean NOT NULL DEFAULT false,
    photo varchar(100),
    town varchar(100),
    candel boolean NOT NULL DEFAULT false,
    lostpwd timestamptz NOT NULL DEFAULT '1970-01-01 00:00:00+00',
    blocked boolean DEFAULT false,
    score integer DEFAULT 0,
    max_score integer DEFAULT 0,
    lastlogin timestamp,
    regdate timestamp DEFAULT now(),
    activated boolean NOT NULL DEFAULT true,
    corrector boolean NOT NULL DEFAULT false,
    userinfo text,
    unread_events integer NOT NULL DEFAULT 0,
    new_email varchar(255),
    style varchar(15) NOT NULL DEFAULT 'tango'
);

CREATE TABLE IF NOT EXISTS msgbase (
    id bigint PRIMARY KEY,
    message text NOT NULL,
    bbcode boolean DEFAULT true
);

CREATE TABLE IF NOT EXISTS topics (
    id integer PRIMARY KEY,
    groupid integer NOT NULL REFERENCES groups(id),
    userid integer NOT NULL REFERENCES users(id),
    title varchar(255) NOT NULL,
    url varchar(255),
    moderate boolean NOT NULL DEFAULT false,
    postdate timestamptz NOT NULL DEFAULT now(),
    linktext varchar(255),
    deleted boolean NOT NULL DEFAULT false,
    stat1 integer NOT NULL DEFAULT 0,
    stat2 integer NOT NULL DEFAULT 0,
    stat3 integer NOT NULL DEFAULT 0,
    stat4 integer NOT NULL DEFAULT 0,
    lastmod timestamptz DEFAULT now(),
    commitby integer,
    notop boolean DEFAULT false,
    commitdate timestamp,
    postscore integer DEFAULT 0,
    postip inet,
    sticky boolean NOT NULL DEFAULT false,
    ua_id integer,
    resolved boolean DEFAULT false,
    minor boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS comments (
    id integer PRIMARY KEY,
    topic integer NOT NULL REFERENCES topics(id),
    userid integer NOT NULL REFERENCES users(id),
    title varchar(255) NOT NULL,
    postdate timestamptz NOT NULL DEFAULT now(),
    replyto integer,
    deleted boolean NOT NULL DEFAULT false,
    postip inet,
    ua_id integer,
    topic_deleted boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS tags_values (
    id serial PRIMARY KEY,
    counter integer DEFAULT 0,
    value varchar(255) NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS tags (
    msgid integer NOT NULL,
    tagid integer NOT NULL REFERENCES tags_values(id),
    PRIMARY KEY(msgid, tagid)
);

CREATE TABLE IF NOT EXISTS user_events (
    id serial PRIMARY KEY,
    userid integer REFERENCES users(id),
    event_date timestamptz NOT NULL DEFAULT now(),
    event_type text NOT NULL DEFAULT 'OTHER',
    message text NOT NULL DEFAULT '',
    topic_id integer REFERENCES topics(id)
);

CREATE INDEX IF NOT EXISTS topics_lastmod_idx ON topics(COALESCE(lastmod, postdate) DESC);
CREATE INDEX IF NOT EXISTS topics_group_idx ON topics(groupid, postdate DESC);
CREATE INDEX IF NOT EXISTS comments_topic_idx ON comments(topic, postdate);
CREATE INDEX IF NOT EXISTS tags_values_lower_idx ON tags_values(lower(value));
CREATE INDEX IF NOT EXISTS topics_fts_idx ON topics USING gin(to_tsvector('russian', title));
