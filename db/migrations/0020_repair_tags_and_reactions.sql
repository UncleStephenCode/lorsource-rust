-- A synonym must always resolve to its canonical tag id. Older Rust code
-- could create a second tags_values row with the synonym text; move any such
-- topic relations back to the canonical id without discarding topics.
INSERT INTO tags(msgid,tagid)
SELECT relation.msgid,synonym.tagid
FROM tags relation
JOIN tags_values duplicate ON duplicate.id=relation.tagid
JOIN tags_synonyms synonym ON lower(synonym.value)=lower(duplicate.value)
WHERE duplicate.id<>synonym.tagid
ON CONFLICT DO NOTHING;

DELETE FROM tags relation
USING tags_values duplicate,tags_synonyms synonym
WHERE relation.tagid=duplicate.id
  AND lower(synonym.value)=lower(duplicate.value)
  AND duplicate.id<>synonym.tagid;

-- `counter` is derived data. Rebuild it from the relation after repairing
-- synonyms and after historical edits that incremented unchanged tags.
UPDATE tags_values value
SET counter=(SELECT count(*)::int FROM tags relation WHERE relation.tagid=value.id);
