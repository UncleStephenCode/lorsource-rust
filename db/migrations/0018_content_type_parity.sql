-- Content section ids/flags from the current Java source.
-- Section.Articles has been 6 since sql/updates/2022-09-15-articles.xml;
-- the first Rust seed incorrectly reused the retired id 4.
INSERT INTO sections(
    id, name, moderate, imagepost, preformat, linktext, havelink, expire,
    vote, add_info, scroll_mode, restrict_score, image_allowed
)
SELECT
    6, name, moderate, imagepost, preformat, linktext, false, expire,
    vote, add_info, scroll_mode, restrict_score, true
FROM sections
WHERE id = 4 AND name = 'Статьи'
ON CONFLICT (id) DO NOTHING;

UPDATE groups
SET section = 6
WHERE section = 4
  AND EXISTS (SELECT 1 FROM sections WHERE id = 4 AND name = 'Статьи')
  AND EXISTS (SELECT 1 FROM sections WHERE id = 6 AND name = 'Статьи');

DELETE FROM sections
WHERE id = 4 AND name = 'Статьи'
  AND EXISTS (SELECT 1 FROM sections WHERE id = 6 AND name = 'Статьи');

-- Preserve the exact current Java column name in migrated databases while
-- keeping image_allowed for the existing Rust queries.
ALTER TABLE sections ADD COLUMN IF NOT EXISTS imageallowed boolean NOT NULL DEFAULT false;
-- A database may originate either from the Java schema (imageallowed) or from
-- an older Rust schema (image_allowed).  Never erase the enabled flag while
-- bringing the two spellings into sync.
UPDATE sections
SET imageallowed = (imageallowed OR image_allowed),
    image_allowed = (imageallowed OR image_allowed)
WHERE imageallowed IS DISTINCT FROM image_allowed;

-- Current Java ImageDao stores only extension/main and derives every file
-- name from the image id.  Early Rust migrations used expanded paths and a
-- primary_image flag instead.  Keep both contracts populated so either
-- implementation can open and create the same topic images.
ALTER TABLE images ADD COLUMN IF NOT EXISTS extension text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS main boolean NOT NULL DEFAULT false;
ALTER TABLE images ADD COLUMN IF NOT EXISTS userid integer REFERENCES users(id);
ALTER TABLE images ADD COLUMN IF NOT EXISTS original text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS medium text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS thumbnail text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS postdate timestamptz NOT NULL DEFAULT now();
ALTER TABLE images ADD COLUMN IF NOT EXISTS width integer;
ALTER TABLE images ADD COLUMN IF NOT EXISTS height integer;
ALTER TABLE images ADD COLUMN IF NOT EXISTS original_name text;
ALTER TABLE images ADD COLUMN IF NOT EXISTS primary_image boolean NOT NULL DEFAULT false;

UPDATE images
SET extension = COALESCE(extension, substring(original from '\\.([^.]+)$'), 'jpg'),
    main = (main OR primary_image),
    primary_image = (main OR primary_image);

UPDATE images
SET original = COALESCE(original, 'images/' || id || '/original.' || extension),
    medium = COALESCE(medium, 'images/' || id || '/1000px.jpg'),
    thumbnail = COALESCE(thumbnail, 'images/' || id || '/500px.jpg');
