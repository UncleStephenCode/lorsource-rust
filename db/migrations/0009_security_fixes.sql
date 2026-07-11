-- Security-fix pass: reconcile schema drift discovered while closing auth
-- gaps in comment/topic moderation and user remarks.
--
-- user_remarks previously used Rust-only column names (userid/who/remark)
-- that never matched the real Java schema (id/user_id/ref_user_id/remark_text,
-- see sql/updates/2012-09-18-user-of-usercomments-table.xml and
-- sql/updates/2026-06-01-user-remarks-unique.xml upstream). Because table
-- creation was guarded by CREATE TABLE IF NOT EXISTS, a Rust-only dev DB
-- ended up with the wrong shape while a real migrated Java DB already has
-- the right one. Reconcile both cases idempotently.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name='user_remarks') THEN
    CREATE TABLE user_remarks (
        id serial PRIMARY KEY,
        user_id integer NOT NULL REFERENCES users(id),
        ref_user_id integer NOT NULL REFERENCES users(id),
        remark_text varchar(255) NOT NULL
    );
  ELSIF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='user_remarks' AND column_name='userid')
    AND NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='user_remarks' AND column_name='user_id') THEN
    ALTER TABLE user_remarks RENAME COLUMN userid TO user_id;
    ALTER TABLE user_remarks RENAME COLUMN who TO ref_user_id;
    ALTER TABLE user_remarks RENAME COLUMN remark TO remark_text;
    ALTER TABLE user_remarks ALTER COLUMN remark_text TYPE varchar(255);
    ALTER TABLE user_remarks DROP CONSTRAINT IF EXISTS user_remarks_pkey;
    ALTER TABLE user_remarks ADD COLUMN id serial;
    ALTER TABLE user_remarks ADD PRIMARY KEY (id);
  END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS user_remarks_userid_refuser ON user_remarks(user_id, ref_user_id);
CREATE INDEX IF NOT EXISTS user_remarks_userid_idx ON user_remarks(user_id);
CREATE INDEX IF NOT EXISTS refuser_remarks_tagid_idx ON user_remarks(ref_user_id);

-- del_info.bonus is part of the upstream schema (sql/updates/2012-01-24-delscore.xml)
-- and is required to record moderator score penalties on comment deletion.
ALTER TABLE del_info ADD COLUMN IF NOT EXISTS bonus integer;

-- b_ips.allow_posting/captcha_required are part of the upstream schema
-- (sql/updates/2011-12-25-ban.xml) and are required by the real /delip.jsp
-- mass-delete-and-ban action (IpBlockDao.blockIP), which the Rust port did
-- not implement at all until this pass.
ALTER TABLE b_ips ADD COLUMN IF NOT EXISTS allow_posting boolean NOT NULL DEFAULT false;
ALTER TABLE b_ips ADD COLUMN IF NOT EXISTS captcha_required boolean NOT NULL DEFAULT true;

-- Java widened this to timestamptz in sql/updates/2014-10-27-timestamp-timezone.xml.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_schema='public' AND table_name='b_ips' AND column_name='ban_date' AND data_type='timestamp without time zone'
  ) THEN
    ALTER TABLE b_ips ALTER COLUMN ban_date TYPE timestamptz USING ban_date AT TIME ZONE 'UTC';
  END IF;
END $$;
