-- Read-only collision check for every canonical sequence that allocates an
-- application primary key.  Nine mappings come from OWNED BY dependencies;
-- four intentionally unowned legacy generators are mapped to the columns used
-- by the current Java DAOs.  The unowned and unused s_guid/s_msg sequences
-- have no canonical application column whose MAX value can be compared.
--
-- Run through psql with tuples-only/unaligned output.  An empty result means
-- every next sequence value is strictly above the current owned column MAX.
\set ON_ERROR_STOP on

WITH expected_mappings(
  sequence_name,
  expected_table,
  expected_column,
  require_owned_by
) AS (
  VALUES
    ('edit_info_id_seq', 'edit_info', 'id', true),
    ('images_id_seq', 'images', 'id', true),
    ('memories_id_seq', 'memories', 'id', true),
    ('message_warnings_id_seq', 'message_warnings', 'id', true),
    ('s_msgid', 'msgbase', 'id', false),
    ('s_uid', 'users', 'id', false),
    ('tags_values_id_seq', 'tags_values', 'id', true),
    ('user_agents_id_seq', 'user_agents', 'id', true),
    ('user_events_id_seq', 'user_events', 'id', true),
    ('user_log_id_seq', 'user_log', 'id', true),
    ('user_remarks_id_seq', 'user_remarks', 'id', true),
    ('vote_id', 'polls', 'id', false),
    ('votes_id', 'polls_variants', 'id', false)
),
mappings AS (
  SELECT
    wanted.sequence_name,
    wanted.expected_table,
    wanted.expected_column,
    wanted.require_owned_by,
    seq_rel.oid AS sequence_oid,
    table_rel.relname AS table_name,
    attr.attname AS column_name,
    seq.seqincrement,
    seq.seqcycle
    FROM expected_mappings AS wanted
    LEFT JOIN pg_catalog.pg_class AS seq_rel
      ON seq_rel.relname = wanted.sequence_name
     AND seq_rel.relnamespace = 'public'::pg_catalog.regnamespace
     AND seq_rel.relkind = 'S'
    LEFT JOIN pg_catalog.pg_sequence AS seq ON seq.seqrelid = seq_rel.oid
    LEFT JOIN pg_catalog.pg_depend AS dep
      ON dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
     AND dep.objid = seq_rel.oid
     AND dep.objsubid = 0
     AND dep.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
     AND dep.deptype IN ('a', 'i')
    LEFT JOIN pg_catalog.pg_class AS table_rel ON table_rel.oid = dep.refobjid
    LEFT JOIN pg_catalog.pg_attribute AS attr
      ON attr.attrelid = dep.refobjid
     AND attr.attnum = dep.refobjsubid
)
SELECT CASE
    WHEN sequence_oid IS NULL
      THEN format(E'missing-sequence\t%s', sequence_name)
    WHEN require_owned_by AND (table_name IS NULL OR column_name IS NULL)
      THEN format(E'missing-owned-by\t%s\t%s.%s', sequence_name, expected_table, expected_column)
    WHEN require_owned_by
      AND (table_name, column_name) <> (expected_table, expected_column)
      THEN format(
        E'changed-owned-by\t%s\texpected=%s.%s\tactual=%s.%s',
        sequence_name,
        expected_table,
        expected_column,
        table_name,
        column_name
      )
    WHEN seqincrement IS DISTINCT FROM 1
      THEN format(E'changed-increment\t%s\texpected=1\tactual=%s', sequence_name, seqincrement)
    WHEN seqcycle IS DISTINCT FROM false
      THEN format(E'changed-cycle\t%s\texpected=false\tactual=%s', sequence_name, seqcycle)
  END
  FROM mappings
 WHERE sequence_oid IS NULL
    OR (require_owned_by AND (table_name IS NULL OR column_name IS NULL))
    OR (
      require_owned_by
      AND (table_name, column_name) <> (expected_table, expected_column)
    )
    OR seqincrement IS DISTINCT FROM 1
    OR seqcycle IS DISTINCT FROM false
 ORDER BY sequence_name;

-- psql's \gexec executes only the generated SELECT statements.  Both the
-- generator and every generated statement are catalog/data reads.
WITH expected_mappings(sequence_name, table_name, column_name) AS (
  VALUES
    ('edit_info_id_seq', 'edit_info', 'id'),
    ('images_id_seq', 'images', 'id'),
    ('memories_id_seq', 'memories', 'id'),
    ('message_warnings_id_seq', 'message_warnings', 'id'),
    ('s_msgid', 'msgbase', 'id'),
    ('s_uid', 'users', 'id'),
    ('tags_values_id_seq', 'tags_values', 'id'),
    ('user_agents_id_seq', 'user_agents', 'id'),
    ('user_events_id_seq', 'user_events', 'id'),
    ('user_log_id_seq', 'user_log', 'id'),
    ('user_remarks_id_seq', 'user_remarks', 'id'),
    ('vote_id', 'polls', 'id'),
    ('votes_id', 'polls_variants', 'id')
),
mappings AS (
  SELECT
    seq_rel.relname AS sequence_name,
    wanted.table_name,
    wanted.column_name,
    seq.seqincrement,
    seq.seqmin,
    seq.seqmax,
    seq.seqcycle
    FROM expected_mappings AS wanted
    JOIN pg_catalog.pg_class AS seq_rel
      ON seq_rel.relname = wanted.sequence_name
     AND seq_rel.relnamespace = 'public'::pg_catalog.regnamespace
     AND seq_rel.relkind = 'S'
    JOIN pg_catalog.pg_sequence AS seq ON seq.seqrelid = seq_rel.oid
)
SELECT format(
  'WITH sequence_candidate(next_value) AS (SELECT CASE WHEN sequence_state.is_called THEN sequence_state.last_value::numeric + %s ELSE sequence_state.last_value::numeric END FROM public.%I AS sequence_state), table_state(max_id) AS (SELECT max(%I)::numeric FROM public.%I) SELECT CASE WHEN sequence_candidate.next_value > %s OR sequence_candidate.next_value < %s THEN %L || sequence_candidate.next_value::text || E''\tmin=%s\tmax=%s'' ELSE %L || sequence_candidate.next_value::text || E''\tmax='' || table_state.max_id::text END FROM sequence_candidate CROSS JOIN table_state WHERE NOT %L::boolean AND (sequence_candidate.next_value > %s OR sequence_candidate.next_value < %s OR (table_state.max_id IS NOT NULL AND sequence_candidate.next_value <= table_state.max_id))',
  seqincrement,
  sequence_name,
  column_name,
  table_name,
  seqmax,
  seqmin,
  format(
    E'exhausted-sequence\t%s\t%s\tnext=',
    sequence_name,
    table_name || '.' || column_name
  ),
  seqmin,
  seqmax,
  format(
    E'unsafe-headroom\t%s\t%s\tnext=',
    sequence_name,
    table_name || '.' || column_name
  ),
  seqcycle,
  seqmax,
  seqmin
)
  FROM mappings
 ORDER BY sequence_name
\gexec
