-- Runtime compatibility fixes for existing dev volumes.
-- Earlier seeds inserted explicit ids into serial tables but did not advance the
-- PostgreSQL sequences.  That makes the first new tag fail with
-- duplicate key value violates unique constraint tags_values_pkey.
SELECT setval(
  pg_get_serial_sequence('tags_values', 'id'),
  GREATEST((SELECT COALESCE(max(id), 0) FROM tags_values), 1),
  true
);

-- Keep the dev admin usable after repeated local migrations.  This is guarded
-- by the dev/noop password and example.test email so it does not overwrite a
-- real migrated Java production account.
UPDATE users
SET activated = true,
    blocked = false,
    canmod = true,
    candel = true,
    corrector = true,
    score = GREATEST(COALESCE(score, 0), 100),
    max_score = GREATEST(COALESCE(max_score, 0), 100)
WHERE lower(nick) = 'admin'
  AND (passwd = '{noop}admin' OR email = 'admin@example.test');

INSERT INTO user_settings(id, settings)
SELECT id, hstore(ARRAY['style','topics','messages','trackerMode'], ARRAY['tango-auto','20','20','all'])
FROM users
WHERE lower(nick) = 'admin'
ON CONFLICT (id) DO NOTHING;
