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
canonical_table_relations AS (
  SELECT c.oid, c.relname, c.relowner, c.relacl, c.relkind
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN canonical_tables AS wanted ON wanted.table_name = c.relname
   WHERE n.nspname = 'public'
     AND c.relkind IN ('r', 'p')
),
canonical_sequence_relations AS (
  SELECT c.oid, c.relname, c.relowner, c.relacl, c.relkind
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
      idx.indisclustered,
      idx.indisvalid,
      idx.indisready,
      idx.indislive,
      idx.indisreplident,
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
    'grant'::text,
    CASE relation.relkind
      WHEN 'S' THEN 'sequence.'
      ELSE 'table.'
    END || relation.relname || '.' ||
      COALESCE(grantee_role.rolname, 'PUBLIC') || '.' || acl.privilege_type,
    jsonb_build_array(acl.is_grantable)::text
    FROM canonical_relations AS relation
    CROSS JOIN LATERAL pg_catalog.aclexplode(
      COALESCE(
        relation.relacl,
        pg_catalog.acldefault(
          CASE relation.relkind
            WHEN 'S' THEN 's'::"char"
            ELSE 'r'::"char"
          END,
          relation.relowner
        )
      )
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
   WHERE acl.grantee = 0
      OR grantee_role.rolname = 'linuxweb'
)
SELECT object_kind, object_identity, object_definition
  FROM object_rows
 ORDER BY object_kind, object_identity, object_definition;
