-- Compatibility with the current Scala/Java source after Liquibase updates.
-- 0003 kept several historic demo-dump table names; the application code in
-- lorsource-java.zip expects the post-migration names and audit/settings tables.

CREATE EXTENSION IF NOT EXISTS hstore;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;

CREATE SEQUENCE IF NOT EXISTS vote_id START WITH 1;
CREATE SEQUENCE IF NOT EXISTS votes_id START WITH 1;

CREATE TABLE IF NOT EXISTS polls (
    id integer PRIMARY KEY DEFAULT nextval('vote_id'),
    topic integer NOT NULL REFERENCES topics(id),
    multiselect boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS polls_variants (
    id integer PRIMARY KEY DEFAULT nextval('votes_id'),
    vote integer NOT NULL REFERENCES polls(id),
    label text NOT NULL,
    votes integer NOT NULL DEFAULT 0
);

-- If the imported demo dump still contains the pre-2011 names, copy the data
-- into the current names used by PollDao.scala. The original tables are left in
-- place so old dumps remain importable.
DO $$
BEGIN
  IF to_regclass('public.votenames') IS NOT NULL THEN
    INSERT INTO polls(id, topic, multiselect)
    SELECT id, topic, multiselect FROM votenames
    ON CONFLICT (id) DO NOTHING;
  END IF;
  IF to_regclass('public.votes') IS NOT NULL THEN
    INSERT INTO polls_variants(id, vote, label, votes)
    SELECT id, vote, label, votes FROM votes
    ON CONFLICT (id) DO NOTHING;
  END IF;
END$$;

ALTER TABLE vote_users ADD COLUMN IF NOT EXISTS variant_id integer;

-- Drop the pre-2011 constraint before rewriting `vote`: old rows stored the
-- selected variant id in vote_users.vote, while current Java stores the poll id
-- in `vote` and the selected answer in `variant_id`.
ALTER TABLE vote_users DROP CONSTRAINT IF EXISTS vote_users_pkey;
ALTER TABLE vote_users DROP CONSTRAINT IF EXISTS vote_users_vote_fkey;
ALTER TABLE vote_users DROP CONSTRAINT IF EXISTS vote_users_variant_id_fkey;

-- Convert old rows where vote_users.vote stored the selected variant id.
DO $$
BEGIN
  IF to_regclass('public.polls_variants') IS NOT NULL
     AND EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='vote_users' AND column_name='variant_id') THEN
    UPDATE vote_users vu
    SET variant_id = vu.vote,
        vote = pv.vote
    FROM polls_variants pv
    WHERE vu.variant_id IS NULL
      AND pv.id = vu.vote;
  END IF;
END$$;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'vote_users_vote_fkey'
  ) THEN
    ALTER TABLE vote_users
      ADD CONSTRAINT vote_users_vote_fkey FOREIGN KEY (vote) REFERENCES polls(id) NOT VALID;
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'vote_users_variant_id_fkey'
  ) THEN
    ALTER TABLE vote_users
      ADD CONSTRAINT vote_users_variant_id_fkey FOREIGN KEY (variant_id) REFERENCES polls_variants(id) NOT VALID;
  END IF;
END$$;

DROP INDEX IF EXISTS vote_users_idx;
CREATE UNIQUE INDEX vote_users_idx ON vote_users(vote, userid, variant_id);
CREATE INDEX IF NOT EXISTS polls_topic_idx ON polls(topic);
CREATE INDEX IF NOT EXISTS polls_variants_vote_idx ON polls_variants(vote);

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'user_log_action') THEN
    CREATE TYPE user_log_action AS ENUM (
      'reset_userpic',
      'set_userpic',
      'block_user',
      'unblock_user',
      'accept_new_email',
      'reset_info',
      'reset_password',
      'set_password',
      'register',
      'score50',
      'set_corrector',
      'unset_corrector',
      'frozen',
      'defrosted',
      'reset_town',
      'reset_url',
      'set_info',
      'sent_password_reset'
    );
  END IF;
END$$;

CREATE TABLE IF NOT EXISTS user_log (
    id serial PRIMARY KEY,
    userid integer NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    action_userid integer NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    action_date timestamptz NOT NULL DEFAULT now(),
    action user_log_action NOT NULL,
    info hstore NOT NULL DEFAULT ''::hstore
);

CREATE INDEX IF NOT EXISTS user_log_userid_idx ON user_log(userid);
CREATE INDEX IF NOT EXISTS user_log_action_userid_idx ON user_log(action_userid);

CREATE TABLE IF NOT EXISTS user_settings (
    id integer PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    settings hstore NOT NULL DEFAULT ''::hstore
);

-- The current Java branch migrated style from users.style into user_settings
-- and then dropped user_settings.main. Keep users.style for older dumps, but
-- mirror it into user_settings for code that expects the new table.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='style') THEN
    INSERT INTO user_settings(id, settings)
    SELECT id, hstore(ARRAY['style'], ARRAY[style])
    FROM users
    WHERE id <> 2
    ON CONFLICT (id) DO NOTHING;
  ELSE
    INSERT INTO user_settings(id, settings)
    SELECT id, ''::hstore
    FROM users
    WHERE id <> 2
    ON CONFLICT (id) DO NOTHING;
  END IF;
END$$;
