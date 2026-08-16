-- Reproducible, read-only catalog query for the structural objects that
-- complement schema-contract.tsv. Run only after manage.sh validate has
-- accepted the vendored Java/Liquibase terminal changeset. With psql's
-- tuples-only, unaligned, tab-separated output the result is the checked-in
-- schema-objects-contract.tsv.
--
-- This deliberately scopes catalog reads to the 33 application tables, 15
-- application sequences and 12 retained application functions. Liquibase's
-- own ledger and extension-owned objects are not part of the runtime contract.
WITH
canonical_tables(table_name) AS (
  VALUES
    ('adv_counts'),
    ('b_ips'),
    ('ban_info'),
    ('comments'),
    ('del_info'),
    ('edit_info'),
    ('email_domains_block'),
    ('groups'),
    ('ignore_list'),
    ('images'),
    ('memories'),
    ('message_warnings'),
    ('monthly_stats'),
    ('msgbase'),
    ('polls'),
    ('polls_variants'),
    ('reactions_log'),
    ('sections'),
    ('tags'),
    ('tags_synonyms'),
    ('tags_values'),
    ('telegram_posts'),
    ('topic_users_notified'),
    ('topics'),
    ('user_agents'),
    ('user_events'),
    ('user_invites'),
    ('user_log'),
    ('user_remarks'),
    ('user_settings'),
    ('user_tags'),
    ('users'),
    ('vote_users')
),
canonical_sequences(sequence_name) AS (
  VALUES
    ('edit_info_id_seq'),
    ('images_id_seq'),
    ('memories_id_seq'),
    ('message_warnings_id_seq'),
    ('s_guid'),
    ('s_msg'),
    ('s_msgid'),
    ('s_uid'),
    ('tags_values_id_seq'),
    ('user_agents_id_seq'),
    ('user_events_id_seq'),
    ('user_log_id_seq'),
    ('user_remarks_id_seq'),
    ('vote_id'),
    ('votes_id')
),
canonical_functions(function_name) AS (
  VALUES
    ('comins'),
    ('create_user_agent'),
    ('get_branch_authors'),
    ('get_title'),
    ('msgdel'),
    ('msgundel'),
    ('new_event'),
    ('normalize_email'),
    ('stat_update'),
    ('stat_update2'),
    ('topins'),
    ('update_monthly_stats')
),
canonical_enum_names(type_name) AS (
  VALUES
    ('edit_event_type'),
    ('event_type'),
    ('markup_type'),
    ('user_log_action'),
    ('warning_type')
),
canonical_table_privileges(table_name, privileges) AS (
  VALUES
    ('adv_counts', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('b_ips', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('ban_info', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('comments', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('del_info', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('edit_info', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('email_domains_block', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('groups', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('ignore_list', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('images', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('memories', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('message_warnings', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('monthly_stats', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('msgbase', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('polls', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('polls_variants', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('reactions_log', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('sections', ARRAY['DELETE', 'SELECT', 'UPDATE']::text[]),
    ('tags', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('tags_synonyms', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('tags_values', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('telegram_posts', ARRAY['DELETE', 'INSERT', 'SELECT']::text[]),
    ('topic_users_notified', ARRAY['DELETE', 'INSERT', 'SELECT']::text[]),
    ('topics', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('user_agents', ARRAY['INSERT', 'SELECT']::text[]),
    ('user_events', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('user_invites', ARRAY['INSERT', 'SELECT', 'UPDATE']::text[]),
    ('user_log', ARRAY['INSERT', 'SELECT']::text[]),
    ('user_remarks', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('user_settings', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('user_tags', ARRAY['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']::text[]),
    ('users', ARRAY['DELETE', 'INSERT', 'SELECT', 'UPDATE']::text[]),
    ('vote_users', ARRAY['INSERT', 'SELECT']::text[])
),
canonical_sequence_privileges(sequence_name, privileges) AS (
  VALUES
    ('edit_info_id_seq', ARRAY['SELECT', 'UPDATE']::text[]),
    ('images_id_seq', ARRAY['UPDATE']::text[]),
    ('memories_id_seq', ARRAY['SELECT', 'UPDATE']::text[]),
    ('s_guid', ARRAY['SELECT', 'UPDATE']::text[]),
    ('s_msgid', ARRAY['SELECT', 'UPDATE', 'USAGE']::text[]),
    ('s_uid', ARRAY['SELECT', 'UPDATE']::text[]),
    ('tags_values_id_seq', ARRAY['UPDATE']::text[]),
    ('user_agents_id_seq', ARRAY['UPDATE']::text[]),
    ('user_events_id_seq', ARRAY['SELECT', 'UPDATE', 'USAGE']::text[]),
    ('user_log_id_seq', ARRAY['USAGE']::text[]),
    ('user_remarks_id_seq', ARRAY['UPDATE']::text[]),
    ('vote_id', ARRAY['UPDATE']::text[]),
    ('votes_id', ARRAY['UPDATE']::text[])
),
canonical_schema AS (
  SELECT n.oid, n.nspname, n.nspowner
    FROM pg_catalog.pg_namespace AS n
   WHERE n.nspname = 'public'
),
canonical_enum_types AS (
  SELECT
    enum_type.oid,
    enum_type.typname,
    enum_type.typowner,
    enum_type.typtype,
    enum_type.typcategory
    FROM pg_catalog.pg_type AS enum_type
    JOIN canonical_schema AS schema ON schema.oid = enum_type.typnamespace
    JOIN canonical_enum_names AS wanted ON wanted.type_name = enum_type.typname
),
canonical_table_relations AS (
  SELECT
    c.oid,
    c.relname,
    c.relowner,
    c.relacl,
    c.relkind,
    c.relpersistence,
    c.relrowsecurity,
    c.relforcerowsecurity,
    c.relam
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN canonical_tables AS wanted ON wanted.table_name = c.relname
   WHERE n.nspname = 'public'
     AND c.relkind IN ('r', 'p')
),
canonical_sequence_relations AS (
  SELECT
    c.oid,
    c.relname,
    c.relowner,
    c.relacl,
    c.relkind,
    c.relpersistence,
    c.relrowsecurity,
    c.relforcerowsecurity,
    c.relam
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN canonical_sequences AS wanted ON wanted.sequence_name = c.relname
   WHERE n.nspname = 'public'
     AND c.relkind = 'S'
),
canonical_relations AS (
  SELECT * FROM canonical_table_relations
  UNION ALL
  SELECT * FROM canonical_sequence_relations
),
object_rows(object_kind, object_identity, object_definition) AS (
  SELECT
    'constraint'::text,
    table_rel.relname || '.' || con.conname,
    jsonb_build_array(
      con.contype::text,
      con.condeferrable,
      con.condeferred,
      con.convalidated,
      pg_catalog.pg_get_constraintdef(con.oid, true)
    )::text
    FROM pg_catalog.pg_constraint AS con
    JOIN canonical_table_relations AS table_rel ON table_rel.oid = con.conrelid

  UNION ALL

  SELECT
    'default'::text,
    table_rel.relname || '.' || attr.attname,
    jsonb_build_array(
      pg_catalog.pg_get_expr(def.adbin, def.adrelid, true)
    )::text
    FROM pg_catalog.pg_attrdef AS def
    JOIN canonical_table_relations AS table_rel ON table_rel.oid = def.adrelid
    JOIN pg_catalog.pg_attribute AS attr
      ON attr.attrelid = def.adrelid
     AND attr.attnum = def.adnum

  UNION ALL

  SELECT
    'index'::text,
    table_rel.relname || '.' || index_rel.relname,
    jsonb_build_array(
      idx.indisunique,
      idx.indisprimary,
      idx.indisexclusion,
      idx.indimmediate,
      idx.indisvalid,
      idx.indisready,
      idx.indislive,
      idx.indnullsnotdistinct,
      idx.indnkeyatts,
      idx.indnatts,
      pg_catalog.pg_get_indexdef(idx.indexrelid, 0, true)
    )::text
    FROM pg_catalog.pg_index AS idx
    JOIN canonical_table_relations AS table_rel ON table_rel.oid = idx.indrelid
    JOIN pg_catalog.pg_class AS index_rel ON index_rel.oid = idx.indexrelid

  UNION ALL

  SELECT
    'relation'::text,
    table_rel.relname,
    jsonb_build_array(
      table_rel.relkind::text,
      table_rel.relpersistence::text,
      table_rel.relrowsecurity,
      table_rel.relforcerowsecurity,
      access_method.amname
    )::text
    FROM canonical_table_relations AS table_rel
    LEFT JOIN pg_catalog.pg_am AS access_method
      ON access_method.oid = table_rel.relam

  UNION ALL

  SELECT
    'schema'::text,
    schema.nspname,
    pg_catalog.jsonb_build_array()::text
    FROM canonical_schema AS schema

  UNION ALL

  SELECT
    'type'::text,
    enum_type.typname,
    pg_catalog.jsonb_build_array(
      enum_type.typtype::text,
      enum_type.typcategory::text
    )::text
    FROM canonical_enum_types AS enum_type

  UNION ALL

  SELECT
    'sequence'::text,
    seq_rel.relname,
    jsonb_build_array(
      pg_catalog.format_type(seq.seqtypid, NULL),
      seq.seqstart,
      seq.seqincrement,
      seq.seqmax,
      seq.seqmin,
      seq.seqcache,
      seq.seqcycle,
      owned_table.relname,
      owned_attr.attname,
      dep.deptype::text
    )::text
    FROM canonical_sequence_relations AS seq_rel
    JOIN pg_catalog.pg_sequence AS seq ON seq.seqrelid = seq_rel.oid
    LEFT JOIN pg_catalog.pg_depend AS dep
      ON dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
     AND dep.objid = seq_rel.oid
     AND dep.objsubid = 0
     AND dep.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
     AND dep.deptype IN ('a', 'i')
    LEFT JOIN pg_catalog.pg_class AS owned_table ON owned_table.oid = dep.refobjid
    LEFT JOIN pg_catalog.pg_attribute AS owned_attr
      ON owned_attr.attrelid = dep.refobjid
     AND owned_attr.attnum = dep.refobjsubid

  UNION ALL

  SELECT
    'function'::text,
    proc.proname || '(' || pg_catalog.pg_get_function_identity_arguments(proc.oid) || ')',
    jsonb_build_array(
      pg_catalog.pg_get_function_result(proc.oid),
      lang.lanname,
      proc.provolatile::text,
      proc.proparallel::text,
      proc.prosecdef,
      proc.proleakproof,
      proc.proisstrict,
      proc.proretset,
      proc.prosrc,
      proc.proconfig
    )::text
    FROM pg_catalog.pg_proc AS proc
    JOIN pg_catalog.pg_namespace AS n ON n.oid = proc.pronamespace
    JOIN pg_catalog.pg_language AS lang ON lang.oid = proc.prolang
    JOIN canonical_functions AS wanted ON wanted.function_name = proc.proname
   WHERE n.nspname = 'public'

  UNION ALL

  SELECT
    'trigger'::text,
    table_rel.relname || '.' || trigger.tgname,
    jsonb_build_array(
      trigger.tgenabled::text,
      pg_catalog.pg_get_triggerdef(trigger.oid, true)
    )::text
    FROM pg_catalog.pg_trigger AS trigger
    JOIN canonical_table_relations AS table_rel ON table_rel.oid = trigger.tgrelid
   WHERE NOT trigger.tgisinternal

  UNION ALL

  SELECT
    'owner'::text,
    CASE relation.relkind
      WHEN 'S' THEN 'sequence.'
      ELSE 'table.'
    END || relation.relname,
    jsonb_build_array(owner_role.rolname)::text
    FROM canonical_relations AS relation
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = relation.relowner

  UNION ALL

  SELECT
    'owner'::text,
    'index.' || table_rel.relname || '.' || index_rel.relname,
    jsonb_build_array(owner_role.rolname)::text
    FROM pg_catalog.pg_index AS idx
    JOIN canonical_table_relations AS table_rel ON table_rel.oid = idx.indrelid
    JOIN pg_catalog.pg_class AS index_rel ON index_rel.oid = idx.indexrelid
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = index_rel.relowner

  UNION ALL

  SELECT
    'owner'::text,
    'function.' || proc.proname || '(' || pg_catalog.pg_get_function_identity_arguments(proc.oid) || ')',
    jsonb_build_array(owner_role.rolname)::text
    FROM pg_catalog.pg_proc AS proc
    JOIN pg_catalog.pg_namespace AS n ON n.oid = proc.pronamespace
    JOIN canonical_functions AS wanted ON wanted.function_name = proc.proname
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = proc.proowner
   WHERE n.nspname = 'public'

  UNION ALL

  SELECT
    'owner'::text,
    'schema.' || schema.nspname,
    pg_catalog.jsonb_build_array(owner_role.rolname)::text
    FROM canonical_schema AS schema
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = schema.nspowner

  UNION ALL

  SELECT
    'owner'::text,
    'type.' || enum_type.typname,
    pg_catalog.jsonb_build_array(owner_role.rolname)::text
    FROM canonical_enum_types AS enum_type
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = enum_type.typowner

  UNION ALL

  SELECT
    'acl'::text,
    CASE relation.relkind
      WHEN 'S' THEN 'sequence.'
      ELSE 'table.'
    END || relation.relname,
    pg_catalog.jsonb_build_array(relation.relacl::text)::text
    FROM canonical_relations AS relation

  UNION ALL

  SELECT
    'acl'::text,
    'function.' || proc.proname || '(' ||
      pg_catalog.pg_get_function_identity_arguments(proc.oid) || ')',
    pg_catalog.jsonb_build_array(proc.proacl::text)::text
    FROM pg_catalog.pg_proc AS proc
    JOIN pg_catalog.pg_namespace AS n ON n.oid = proc.pronamespace
    JOIN canonical_functions AS wanted ON wanted.function_name = proc.proname
   WHERE n.nspname = 'public'

  UNION ALL

  SELECT
    'grant'::text,
    'table.' || table_rel.relname || '.linuxweb.' || privilege.privilege,
    pg_catalog.jsonb_build_array(
      pg_catalog.has_table_privilege('linuxweb', table_rel.oid, privilege.privilege)
    )::text
    FROM canonical_table_relations AS table_rel
    JOIN canonical_table_privileges AS wanted
      ON wanted.table_name = table_rel.relname
    CROSS JOIN LATERAL unnest(wanted.privileges) AS privilege(privilege)

  UNION ALL

  SELECT
    'grant'::text,
    'sequence.' || seq_rel.relname || '.linuxweb.' || privilege.privilege,
    pg_catalog.jsonb_build_array(
      pg_catalog.has_sequence_privilege('linuxweb', seq_rel.oid, privilege.privilege)
    )::text
    FROM canonical_sequence_relations AS seq_rel
    JOIN canonical_sequence_privileges AS wanted
      ON wanted.sequence_name = seq_rel.relname
    CROSS JOIN LATERAL unnest(wanted.privileges) AS privilege(privilege)

  UNION ALL

  SELECT
    'grant'::text,
    'function.' || proc.proname || '(' ||
      pg_catalog.pg_get_function_identity_arguments(proc.oid) ||
      ').linuxweb.EXECUTE',
    pg_catalog.jsonb_build_array(
      pg_catalog.has_function_privilege('linuxweb', proc.oid, 'EXECUTE')
    )::text
    FROM pg_catalog.pg_proc AS proc
    JOIN pg_catalog.pg_namespace AS n ON n.oid = proc.pronamespace
    JOIN canonical_functions AS wanted ON wanted.function_name = proc.proname
   WHERE n.nspname = 'public'

  UNION ALL

  SELECT
    'grant'::text,
    'schema.' || schema.nspname || '.linuxweb.USAGE',
    pg_catalog.jsonb_build_array(
      pg_catalog.has_schema_privilege('linuxweb', schema.oid, 'USAGE')
    )::text
    FROM canonical_schema AS schema

  UNION ALL

  SELECT
    'grant'::text,
    'type.' || enum_type.typname || '.linuxweb.USAGE',
    pg_catalog.jsonb_build_array(
      pg_catalog.has_type_privilege('linuxweb', enum_type.oid, 'USAGE')
    )::text
    FROM canonical_enum_types AS enum_type
)
SELECT object_kind, object_identity, object_definition
  FROM object_rows
 ORDER BY object_kind, object_identity, object_definition;
