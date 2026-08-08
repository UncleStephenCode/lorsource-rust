-- Current Java stores the exact MarkupType id. Preserve legacy rows while
-- making new/default rows readable by both implementations.
ALTER TABLE msgbase ALTER COLUMN markup SET DEFAULT 'BBCODE_TEX';

UPDATE msgbase
SET markup = CASE
    WHEN markup = 'LORCODE' AND bbcode = false THEN 'MARKDOWN'
    WHEN markup = 'LORCODE' THEN 'BBCODE_TEX'
    ELSE markup
END;
