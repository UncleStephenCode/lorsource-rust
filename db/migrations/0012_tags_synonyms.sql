-- tags_synonyms previously used Rust-only column names/PK strategy
-- (id serial PK, tag_id, synonym) that don't match the real Java schema
-- (value text PRIMARY KEY, tagid int) - see
-- sql/updates/2023-01-28-tag-synonyms.xml upstream. Reconcile idempotently,
-- same pattern as the user_remarks fix in 0009.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name='tags_synonyms') THEN
    CREATE TABLE tags_synonyms (
        value text PRIMARY KEY,
        tagid integer NOT NULL REFERENCES tags_values(id)
    );
  ELSIF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='tags_synonyms' AND column_name='synonym')
    AND NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='tags_synonyms' AND column_name='value') THEN
    ALTER TABLE tags_synonyms DROP CONSTRAINT IF EXISTS tags_synonyms_pkey;
    ALTER TABLE tags_synonyms DROP COLUMN IF EXISTS id;
    ALTER TABLE tags_synonyms RENAME COLUMN synonym TO value;
    ALTER TABLE tags_synonyms RENAME COLUMN tag_id TO tagid;
    ALTER TABLE tags_synonyms ADD PRIMARY KEY (value);
  END IF;
END $$;
