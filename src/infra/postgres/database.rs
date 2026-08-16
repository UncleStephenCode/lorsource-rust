use anyhow::Context;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tracing::{info, warn};

pub type TyPgPool = PgPool;

const S_SCHEMA_CONTRACT: &str = include_str!("../../../compat/java-db/schema-contract.tsv");
const S_SCHEMA_OBJECTS_CONTRACT: &str =
    include_str!("../../../compat/java-db/schema-objects-contract.tsv");
const S_SCHEMA_OBJECTS_QUERY: &str =
    include_str!("../../../compat/java-db/export-schema-objects.sql");

const VEC_SCHEMA_OBJECT_KINDS: &[&str] = &[
    "acl",
    "constraint",
    "default",
    "function",
    "grant",
    "index",
    "owner",
    "relation",
    "schema",
    "sequence",
    "trigger",
    "type",
];

/// Owner role names are deployment metadata rather than application schema
/// semantics. A different migration-owner role is reported in the fingerprint
/// drift, but it does not make an otherwise compatible database unusable.
const VEC_ADVISORY_SCHEMA_OBJECT_KINDS: &[&str] = &["acl", "owner"];

const I_SCHEMA_DRIFT_LOG_LIMIT: usize = 25;
const I_SCHEMA_ERROR_LIMIT: usize = 50;

const VEC_REQUIRED_EXTENSIONS: &[&str] = &["fuzzystrmatch", "hstore"];

const VEC_REQUIRED_FUNCTIONS: &[&str] = &[
    "comins",
    "create_user_agent",
    "get_branch_authors",
    "get_title",
    "msgdel",
    "msgundel",
    "new_event",
    "normalize_email",
    "stat_update",
    "stat_update2",
    "topins",
    "update_monthly_stats",
];

const VEC_REQUIRED_SEQUENCES: &[&str] = &[
    "edit_info_id_seq",
    "images_id_seq",
    "memories_id_seq",
    "message_warnings_id_seq",
    "s_guid",
    "s_msg",
    "s_msgid",
    "s_uid",
    "tags_values_id_seq",
    "user_agents_id_seq",
    "user_events_id_seq",
    "user_log_id_seq",
    "user_remarks_id_seq",
    "vote_id",
    "votes_id",
];

const VEC_REQUIRED_TRIGGERS: &[(&str, &str)] = &[
    ("comments", "comins_t"),
    ("del_info", "msgdel_t"),
    ("del_info", "msgundel_t"),
    ("topics", "topins_t"),
    ("user_events", "new_event_t"),
];

const VEC_REQUIRED_ENUMS: &[(&str, &[&str])] = &[
    ("edit_event_type", &["TOPIC", "COMMENT"]),
    (
        "event_type",
        &[
            "WATCH", "REPLY", "DEL", "OTHER", "REF", "TAG", "REACTION", "WARNING",
        ],
    ),
    (
        "markup_type",
        &["PLAIN", "BBCODE_TEX", "BBCODE_ULB", "MARKDOWN"],
    ),
    (
        "user_log_action",
        &[
            "reset_userpic",
            "set_userpic",
            "block_user",
            "unblock_user",
            "accept_new_email",
            "reset_info",
            "reset_password",
            "set_password",
            "register",
            "score50",
            "set_corrector",
            "unset_corrector",
            "frozen",
            "defrosted",
            "reset_town",
            "reset_url",
            "set_info",
            "sent_password_reset",
        ],
    ),
    ("warning_type", &["rule", "tag", "spelling", "group"]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnDatabaseKind {
    Empty,
    JavaLiquibase,
    LegacyRust,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct StDatabaseFingerprint {
    bHasUsers: bool,
    bHasTopics: bool,
    bHasLiquibaseLedger: bool,
    bHasSqlxLedger: bool,
    bHasLegacyRustColumns: bool,
    iBusinessTableCount: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StExpectedColumn {
    sTypeName: &'static str,
    bNotNull: bool,
}

type TySchemaObjectKey = (String, String);
type TySchemaObjectMap = BTreeMap<TySchemaObjectKey, String>;

#[derive(Debug, Default, PartialEq, Eq)]
struct StSchemaObjectComparison {
    vecBlockingProblems: Vec<String>,
    vecAdvisoryDrift: Vec<String>,
}

pub async fn oConnect(sDatabaseUrl: &str) -> anyhow::Result<TyPgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(sDatabaseUrl)
        .await
        // Never include the connection URL here: anyhow's context is logged
        // during startup and a normal PostgreSQL URL commonly contains the
        // runtime password.
        .context("failed to connect to PostgreSQL")
}

/// Validate the Java/Liquibase schema without mutating it.
///
/// The `linuxweb` runtime role intentionally cannot read Liquibase's ledger.
/// Runtime validation therefore uses catalog metadata only. Operators must run
/// `compat/java-db/manage.sh validate` with the migration owner to validate the
/// terminal changeset and Liquibase checksums.
pub async fn vVerifySchema(oPool: &TyPgPool) -> anyhow::Result<()> {
    let stFingerprint = stReadDatabaseFingerprint(oPool).await?;
    let enKind = enClassifyDatabase(stFingerprint);

    match enKind {
        EnDatabaseKind::JavaLiquibase => {}
        EnDatabaseKind::Empty => anyhow::bail!(
            "refusing to start against an empty PostgreSQL database; bootstrap the canonical Java schema with `LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db compat/java-db/manage.sh bootstrap`"
        ),
        EnDatabaseKind::LegacyRust => anyhow::bail!(
            "refusing to start against a legacy Rust/SQLx schema; it is not migration-compatible with the current Java database contract"
        ),
        EnDatabaseKind::Mixed => anyhow::bail!(
            "refusing to start against a mixed Liquibase/SQLx or Java/legacy-Rust schema; restore a clean Java database clone and validate it before cutover"
        ),
        EnDatabaseKind::Unknown => anyhow::bail!(
            "refusing to start against an unknown PostgreSQL schema; a current Java/Liquibase database is required"
        ),
    }

    vVerifyColumns(oPool).await?;
    vVerifyExtensions(oPool).await?;
    vVerifyEnums(oPool).await?;
    vVerifySequences(oPool).await?;
    vVerifyFunctions(oPool).await?;
    vVerifyTriggers(oPool).await?;
    vVerifySchemaObjects(oPool).await?;

    info!(
        "validated current Java database structure and canonical schema-object contract without reading or changing the Liquibase ledger"
    );
    Ok(())
}

fn enClassifyDatabase(stFingerprint: StDatabaseFingerprint) -> EnDatabaseKind {
    let bJavaMarkers = stFingerprint.bHasUsers && stFingerprint.bHasTopics;
    let bLegacyMarkers = stFingerprint.bHasSqlxLedger || stFingerprint.bHasLegacyRustColumns;

    if stFingerprint.bHasLiquibaseLedger && bLegacyMarkers {
        EnDatabaseKind::Mixed
    } else if stFingerprint.bHasLiquibaseLedger && bJavaMarkers {
        EnDatabaseKind::JavaLiquibase
    } else if stFingerprint.bHasLiquibaseLedger {
        EnDatabaseKind::Unknown
    } else if bLegacyMarkers {
        EnDatabaseKind::LegacyRust
    } else if stFingerprint.iBusinessTableCount == 0 {
        EnDatabaseKind::Empty
    } else {
        EnDatabaseKind::Unknown
    }
}

async fn stReadDatabaseFingerprint(oPool: &TyPgPool) -> anyhow::Result<StDatabaseFingerprint> {
    let stRow = sqlx::query(
        r#"
        SELECT
          to_regclass('public.users') IS NOT NULL AS has_users,
          to_regclass('public.topics') IS NOT NULL AS has_topics,
          to_regclass('public.databasechangelog') IS NOT NULL AS has_liquibase_ledger,
          to_regclass('public._sqlx_migrations') IS NOT NULL AS has_sqlx_ledger,
          EXISTS (
            SELECT 1
              FROM pg_catalog.pg_attribute AS a
              JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
              JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public'
               AND a.attnum > 0
               AND NOT a.attisdropped
               AND (c.relname, a.attname) IN (
                 ('users', 'style'),
                 ('users', 'settings'),
                 ('users', 'force_unlogin'),
                 ('topics', 'stat2'),
                 ('topics', 'stat4'),
                 ('topics', 'no_comments'),
                 ('topics', 'image'),
                 ('topics', 'warning_counter'),
                 ('topics', 'score_loss'),
                 ('comments', 'editor'),
                 ('comments', 'editdate'),
                 ('comments', 'topic_deleted'),
                 ('sections', 'preformat'),
                 ('sections', 'add_info'),
                 ('sections', 'image_allowed'),
                 ('groups', 'stat1'),
                 ('groups', 'stat2'),
                 ('groups', 'stat4'),
                 ('images', 'userid'),
                 ('images', 'filename'),
                 ('adv_counts', 'id'),
                 ('reactions_log', 'id'),
                 ('reactions_log', 'msgid')
               )
          ) AS has_legacy_rust_columns,
          (
            SELECT count(*)::bigint
              FROM pg_catalog.pg_class AS c
              JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public'
               AND c.relkind IN ('r', 'p')
               AND c.relname NOT IN ('databasechangelog', 'databasechangeloglock')
          ) AS business_table_count
        "#,
    )
    .fetch_one(oPool)
    .await
    .context("failed to classify PostgreSQL schema")?;

    Ok(StDatabaseFingerprint {
        bHasUsers: stRow.try_get("has_users")?,
        bHasTopics: stRow.try_get("has_topics")?,
        bHasLiquibaseLedger: stRow.try_get("has_liquibase_ledger")?,
        bHasSqlxLedger: stRow.try_get("has_sqlx_ledger")?,
        bHasLegacyRustColumns: stRow.try_get("has_legacy_rust_columns")?,
        iBusinessTableCount: stRow.try_get("business_table_count")?,
    })
}

fn mapExpectedColumns() -> anyhow::Result<BTreeMap<(&'static str, &'static str), StExpectedColumn>>
{
    let mut mapColumns = BTreeMap::new();

    for (iLine, sLine) in S_SCHEMA_CONTRACT.lines().enumerate() {
        let mut itFields = sLine.split('\t');
        let sTable = itFields.next().context("missing table name")?;
        let sColumn = itFields.next().context("missing column name")?;
        let sTypeName = itFields.next().context("missing PostgreSQL type")?;
        let sNullable = itFields.next().context("missing nullable marker")?;
        anyhow::ensure!(
            itFields.next().is_none(),
            "invalid schema contract line {}",
            iLine + 1
        );
        let bNotNull = match sNullable {
            "NO" => true,
            "YES" => false,
            _ => anyhow::bail!(
                "invalid nullable marker on schema contract line {}",
                iLine + 1
            ),
        };
        let optPrevious = mapColumns.insert(
            (sTable, sColumn),
            StExpectedColumn {
                sTypeName,
                bNotNull,
            },
        );
        anyhow::ensure!(
            optPrevious.is_none(),
            "duplicate schema contract entry {sTable}.{sColumn}"
        );
    }

    Ok(mapColumns)
}

async fn vVerifyColumns(oPool: &TyPgPool) -> anyhow::Result<()> {
    let mapExpected = mapExpectedColumns()?;
    let vecTables: Vec<String> = mapExpected
        .keys()
        .map(|(sTable, _)| (*sTable).to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let vecRows = sqlx::query(
        r#"
        SELECT
          c.relname::text AS table_name,
          a.attname::text AS column_name,
          t.typname::text AS type_name,
          a.attnotnull AS not_null
          FROM pg_catalog.pg_attribute AS a
          JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
          JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
          JOIN pg_catalog.pg_type AS t ON t.oid = a.atttypid
         WHERE n.nspname = 'public'
           AND c.relname::text = ANY($1::text[])
           AND c.relkind IN ('r', 'p')
           AND a.attnum > 0
           AND NOT a.attisdropped
         ORDER BY c.relname, a.attnum
        "#,
    )
    .bind(&vecTables)
    .fetch_all(oPool)
    .await
    .context("failed to inspect Java table columns")?;

    let mut mapActual = BTreeMap::new();
    for stRow in vecRows {
        let sTable: String = stRow.try_get("table_name")?;
        let sColumn: String = stRow.try_get("column_name")?;
        let sTypeName: String = stRow.try_get("type_name")?;
        let bNotNull: bool = stRow.try_get("not_null")?;
        mapActual.insert((sTable, sColumn), (sTypeName, bNotNull));
    }

    let mut vecProblems = Vec::new();
    for ((sTable, sColumn), stExpected) in &mapExpected {
        match mapActual.get(&(sTable.to_string(), sColumn.to_string())) {
            None => vecProblems.push(format!("missing column {sTable}.{sColumn}")),
            Some((sActualType, bActualNotNull))
                if sActualType != stExpected.sTypeName
                    || *bActualNotNull != stExpected.bNotNull =>
            {
                vecProblems.push(format!(
                    "{sTable}.{sColumn}: expected type={} not_null={}, found type={} not_null={}",
                    stExpected.sTypeName, stExpected.bNotNull, sActualType, bActualNotNull
                ));
            }
            Some(_) => {}
        }
    }
    for (sTable, sColumn) in mapActual.keys() {
        if !mapExpected.contains_key(&(sTable.as_str(), sColumn.as_str())) {
            vecProblems.push(format!("unexpected column {sTable}.{sColumn}"));
        }
    }

    anyhow::ensure!(
        vecProblems.is_empty(),
        "PostgreSQL columns do not match the vendored Java schema:\n- {}",
        vecProblems.join("\n- ")
    );
    Ok(())
}

async fn vVerifyExtensions(oPool: &TyPgPool) -> anyhow::Result<()> {
    let vecExpected: Vec<String> = VEC_REQUIRED_EXTENSIONS
        .iter()
        .map(|sName| (*sName).to_string())
        .collect();
    let setActual: BTreeSet<String> = sqlx::query_scalar(
        "SELECT extname::text FROM pg_catalog.pg_extension WHERE extname::text = ANY($1::text[])",
    )
    .bind(&vecExpected)
    .fetch_all(oPool)
    .await?
    .into_iter()
    .collect();
    let setExpected: BTreeSet<String> = vecExpected.into_iter().collect();
    anyhow::ensure!(
        setActual == setExpected,
        "required PostgreSQL extensions are missing: expected {setExpected:?}, found {setActual:?}"
    );
    Ok(())
}

async fn vVerifyEnums(oPool: &TyPgPool) -> anyhow::Result<()> {
    let vecNames: Vec<String> = VEC_REQUIRED_ENUMS
        .iter()
        .map(|(sName, _)| (*sName).to_string())
        .collect();
    let vecRows = sqlx::query(
        r#"
        SELECT t.typname::text AS type_name, e.enumlabel::text AS enum_label
          FROM pg_catalog.pg_type AS t
          JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
          JOIN pg_catalog.pg_enum AS e ON e.enumtypid = t.oid
         WHERE n.nspname = 'public'
           AND t.typname::text = ANY($1::text[])
         ORDER BY t.typname, e.enumsortorder
        "#,
    )
    .bind(&vecNames)
    .fetch_all(oPool)
    .await?;
    let mut mapActual: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for stRow in vecRows {
        mapActual
            .entry(stRow.try_get("type_name")?)
            .or_default()
            .push(stRow.try_get("enum_label")?);
    }

    for (sName, vecLabels) in VEC_REQUIRED_ENUMS {
        let vecExpected: Vec<String> = vecLabels
            .iter()
            .map(|sLabel| (*sLabel).to_string())
            .collect();
        anyhow::ensure!(
            mapActual.get(*sName) == Some(&vecExpected),
            "PostgreSQL enum {sName} does not match the current Java schema: expected {vecExpected:?}, found {:?}",
            mapActual.get(*sName)
        );
    }
    Ok(())
}

async fn vVerifySequences(oPool: &TyPgPool) -> anyhow::Result<()> {
    let vecExpected: Vec<String> = VEC_REQUIRED_SEQUENCES
        .iter()
        .map(|sName| (*sName).to_string())
        .collect();
    let setActual: BTreeSet<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname::text
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind = 'S'
           AND c.relname::text = ANY($1::text[])
        "#,
    )
    .bind(&vecExpected)
    .fetch_all(oPool)
    .await?
    .into_iter()
    .collect();
    let setExpected: BTreeSet<String> = vecExpected.into_iter().collect();
    anyhow::ensure!(
        setActual == setExpected,
        "required PostgreSQL sequences are missing: expected {setExpected:?}, found {setActual:?}"
    );
    Ok(())
}

async fn vVerifyFunctions(oPool: &TyPgPool) -> anyhow::Result<()> {
    let vecExpected: Vec<String> = VEC_REQUIRED_FUNCTIONS
        .iter()
        .map(|sName| (*sName).to_string())
        .collect();
    let setActual: BTreeSet<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT p.proname::text
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname::text = ANY($1::text[])
        "#,
    )
    .bind(&vecExpected)
    .fetch_all(oPool)
    .await?
    .into_iter()
    .collect();
    let setExpected: BTreeSet<String> = vecExpected.into_iter().collect();
    anyhow::ensure!(
        setActual == setExpected,
        "required Java PostgreSQL functions are missing: expected {setExpected:?}, found {setActual:?}"
    );
    Ok(())
}

async fn vVerifyTriggers(oPool: &TyPgPool) -> anyhow::Result<()> {
    let vecRows = sqlx::query(
        r#"
        SELECT c.relname::text AS table_name, t.tgname::text AS trigger_name
          FROM pg_catalog.pg_trigger AS t
          JOIN pg_catalog.pg_class AS c ON c.oid = t.tgrelid
          JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND NOT t.tgisinternal
           AND t.tgenabled IN ('O', 'A')
        "#,
    )
    .fetch_all(oPool)
    .await?;
    let setActual: BTreeSet<(String, String)> = vecRows
        .into_iter()
        .map(|stRow| Ok((stRow.try_get("table_name")?, stRow.try_get("trigger_name")?)))
        .collect::<Result<_, sqlx::Error>>()?;

    for (sTable, sTrigger) in VEC_REQUIRED_TRIGGERS {
        anyhow::ensure!(
            setActual.contains(&(sTable.to_string(), sTrigger.to_string())),
            "required Java trigger {sTable}.{sTrigger} is missing"
        );
    }
    Ok(())
}

fn mapExpectedSchemaObjects() -> anyhow::Result<TySchemaObjectMap> {
    let mut mapObjects = BTreeMap::new();

    for (iLine, sLine) in S_SCHEMA_OBJECTS_CONTRACT.lines().enumerate() {
        let mut itFields = sLine.splitn(3, '\t');
        let sKind = itFields.next().context("missing schema-object kind")?;
        let sIdentity = itFields.next().context("missing schema-object identity")?;
        let sDefinition = itFields
            .next()
            .context("missing schema-object definition")?;

        anyhow::ensure!(
            VEC_SCHEMA_OBJECT_KINDS.contains(&sKind),
            "unknown schema-object kind {sKind:?} on contract line {}",
            iLine + 1
        );
        anyhow::ensure!(
            !sIdentity.is_empty(),
            "empty schema-object identity on contract line {}",
            iLine + 1
        );
        let stDefinition: serde_json::Value =
            serde_json::from_str(sDefinition).with_context(|| {
                format!("invalid schema-object JSON on contract line {}", iLine + 1)
            })?;
        let sCanonicalDefinition = serde_json::to_string(&stDefinition)?;
        let optPrevious = mapObjects.insert(
            (sKind.to_string(), sIdentity.to_string()),
            sCanonicalDefinition,
        );
        anyhow::ensure!(
            optPrevious.is_none(),
            "duplicate schema-object contract entry {sKind}.{sIdentity}"
        );
    }

    anyhow::ensure!(
        !mapObjects.is_empty(),
        "schema-object contract must not be empty"
    );
    Ok(mapObjects)
}

async fn mapReadSchemaObjects(oPool: &TyPgPool) -> anyhow::Result<TySchemaObjectMap> {
    let vecRows = sqlx::query(S_SCHEMA_OBJECTS_QUERY)
        .fetch_all(oPool)
        .await
        .context("failed to inspect canonical Java schema objects")?;
    let mut mapObjects = BTreeMap::new();

    for stRow in vecRows {
        let sKind: String = stRow.try_get("object_kind")?;
        let sIdentity: String = stRow.try_get("object_identity")?;
        let sDefinition: String = stRow.try_get("object_definition")?;
        anyhow::ensure!(
            VEC_SCHEMA_OBJECT_KINDS.contains(&sKind.as_str()),
            "catalog query returned unknown schema-object kind {sKind:?}"
        );
        let stDefinition: serde_json::Value = serde_json::from_str(&sDefinition)
            .with_context(|| format!("catalog returned invalid JSON for {sKind}.{sIdentity}"))?;
        let sCanonicalDefinition = serde_json::to_string(&stDefinition)?;
        let optPrevious =
            mapObjects.insert((sKind.clone(), sIdentity.clone()), sCanonicalDefinition);
        anyhow::ensure!(
            optPrevious.is_none(),
            "catalog returned duplicate schema object {sKind}.{sIdentity}"
        );
    }

    Ok(mapObjects)
}

fn bSchemaObjectKindIsAdvisory(sKind: &str) -> bool {
    VEC_ADVISORY_SCHEMA_OBJECT_KINDS.contains(&sKind)
}

fn bAdditionalSchemaObjectIsBlocking(sKind: &str) -> bool {
    matches!(sKind, "constraint" | "trigger")
}

fn stCompareSchemaObjects(
    mapExpected: &TySchemaObjectMap,
    mapActual: &TySchemaObjectMap,
) -> StSchemaObjectComparison {
    let mut stComparison = StSchemaObjectComparison::default();

    for ((sKind, sIdentity), sExpectedDefinition) in mapExpected {
        let optActualDefinition = mapActual.get(&(sKind.clone(), sIdentity.clone()));
        let optProblem = match optActualDefinition {
            None => Some(format!("missing {sKind} {sIdentity}")),
            Some(sActualDefinition) if sActualDefinition != sExpectedDefinition => Some(format!(
                "changed {sKind} {sIdentity}: expected {sExpectedDefinition}, found {sActualDefinition}"
            )),
            Some(_) => None,
        };

        if let Some(sProblem) = optProblem {
            if bSchemaObjectKindIsAdvisory(sKind) {
                stComparison.vecAdvisoryDrift.push(sProblem);
            } else {
                stComparison.vecBlockingProblems.push(sProblem);
            }
        }
    }

    // Additional constraints and enabled triggers can reject, rewrite or add
    // effects to otherwise valid Java writes.  Other bounded operator objects
    // remain visible in the fingerprint without preventing startup.
    for (sKind, sIdentity) in mapActual.keys() {
        if !mapExpected.contains_key(&(sKind.clone(), sIdentity.clone())) {
            let sProblem = format!("additional {sKind} {sIdentity}");
            if bAdditionalSchemaObjectIsBlocking(sKind) {
                stComparison.vecBlockingProblems.push(sProblem);
            } else {
                stComparison.vecAdvisoryDrift.push(sProblem);
            }
        }
    }

    stComparison
}

fn sSchemaObjectFingerprint(mapObjects: &TySchemaObjectMap) -> String {
    use std::fmt::Write as _;

    let mut stDigest = Sha256::new();
    for ((sKind, sIdentity), sDefinition) in mapObjects {
        stDigest.update(sKind.as_bytes());
        stDigest.update([0]);
        stDigest.update(sIdentity.as_bytes());
        stDigest.update([0]);
        stDigest.update(sDefinition.as_bytes());
        stDigest.update(b"\n");
    }
    let arrDigest = stDigest.finalize();
    let mut sHex = String::with_capacity(arrDigest.len() * 2);
    for iByte in arrDigest {
        write!(&mut sHex, "{iByte:02x}").expect("writing to String cannot fail");
    }
    sHex
}

fn sLimitedProblems(vecProblems: &[String], iLimit: usize) -> String {
    let mut vecVisible: Vec<&str> = vecProblems
        .iter()
        .take(iLimit)
        .map(String::as_str)
        .collect();
    if vecProblems.len() > iLimit {
        vecVisible.push("(additional differences omitted)");
    }
    vecVisible.join("\n- ")
}

async fn vVerifySchemaObjects(oPool: &TyPgPool) -> anyhow::Result<()> {
    let mapExpected = mapExpectedSchemaObjects()?;
    let mapActual = mapReadSchemaObjects(oPool).await?;
    let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
    let sExpectedFingerprint = sSchemaObjectFingerprint(&mapExpected);
    let sActualFingerprint = sSchemaObjectFingerprint(&mapActual);

    info!(
        expected_objects = mapExpected.len(),
        actual_objects = mapActual.len(),
        expected_sha256 = %sExpectedFingerprint,
        actual_sha256 = %sActualFingerprint,
        "computed bounded Java schema-object fingerprint"
    );

    if !stComparison.vecAdvisoryDrift.is_empty() {
        warn!(
            drift_count = stComparison.vecAdvisoryDrift.len(),
            drift = %sLimitedProblems(
                &stComparison.vecAdvisoryDrift,
                I_SCHEMA_DRIFT_LOG_LIMIT
            ),
            "PostgreSQL schema has non-blocking drift from the canonical Java object fingerprint"
        );
    }

    anyhow::ensure!(
        stComparison.vecBlockingProblems.is_empty(),
        "PostgreSQL schema objects do not satisfy the vendored Java/Liquibase contract ({} blocking differences):\n- {}",
        stComparison.vecBlockingProblems.len(),
        sLimitedProblems(&stComparison.vecBlockingProblems, I_SCHEMA_ERROR_LIMIT)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stFingerprint() -> StDatabaseFingerprint {
        StDatabaseFingerprint {
            bHasUsers: true,
            bHasTopics: true,
            bHasLiquibaseLedger: true,
            bHasSqlxLedger: false,
            bHasLegacyRustColumns: false,
            iBusinessTableCount: 33,
        }
    }

    #[test]
    fn classifiesOnlyCleanJavaDatabaseAsCompatible() {
        assert_eq!(
            enClassifyDatabase(stFingerprint()),
            EnDatabaseKind::JavaLiquibase
        );

        let mut stMixed = stFingerprint();
        stMixed.bHasSqlxLedger = true;
        assert_eq!(enClassifyDatabase(stMixed), EnDatabaseKind::Mixed);

        let mut stLegacyColumns = stFingerprint();
        stLegacyColumns.bHasLegacyRustColumns = true;
        assert_eq!(enClassifyDatabase(stLegacyColumns), EnDatabaseKind::Mixed);
    }

    #[test]
    fn failsClosedForEmptyLegacyAndUnknownDatabases() {
        let mut stEmpty = stFingerprint();
        stEmpty.bHasUsers = false;
        stEmpty.bHasTopics = false;
        stEmpty.bHasLiquibaseLedger = false;
        stEmpty.iBusinessTableCount = 0;
        assert_eq!(enClassifyDatabase(stEmpty), EnDatabaseKind::Empty);

        let mut stLegacy = stEmpty;
        stLegacy.bHasSqlxLedger = true;
        assert_eq!(enClassifyDatabase(stLegacy), EnDatabaseKind::LegacyRust);

        let mut stUnknown = stEmpty;
        stUnknown.iBusinessTableCount = 1;
        assert_eq!(enClassifyDatabase(stUnknown), EnDatabaseKind::Unknown);
    }

    #[test]
    fn parsesCompleteCanonicalColumnContract() {
        let mapColumns = mapExpectedColumns().expect("schema contract must parse");
        let setTables: BTreeSet<_> = mapColumns.keys().map(|(sTable, _)| *sTable).collect();
        assert_eq!(setTables.len(), 33);
        assert_eq!(mapColumns.len(), 214);
        assert_eq!(
            mapColumns.get(&("comments", "editor_id")),
            Some(&StExpectedColumn {
                sTypeName: "int4",
                bNotNull: false
            })
        );
        assert!(!mapColumns.contains_key(&("comments", "editor")));
        assert!(!mapColumns.contains_key(&("topics", "stat2")));
    }

    #[test]
    fn parsesCompleteCanonicalSchemaObjectContract() {
        let mapObjects = mapExpectedSchemaObjects().expect("schema-object contract must parse");
        let mut mapCounts = BTreeMap::<String, usize>::new();
        for (sKind, _) in mapObjects.keys() {
            *mapCounts.entry(sKind.clone()).or_default() += 1;
        }

        assert_eq!(mapObjects.len(), 728);
        assert_eq!(mapCounts.get("acl"), Some(&60));
        assert_eq!(mapCounts.get("constraint"), Some(&82));
        assert_eq!(mapCounts.get("default"), Some(&61));
        assert_eq!(mapCounts.get("function"), Some(&12));
        assert_eq!(mapCounts.get("grant"), Some(&186));
        assert_eq!(mapCounts.get("index"), Some(&101));
        assert_eq!(mapCounts.get("owner"), Some(&167));
        assert_eq!(mapCounts.get("relation"), Some(&33));
        assert_eq!(mapCounts.get("schema"), Some(&1));
        assert_eq!(mapCounts.get("sequence"), Some(&15));
        assert_eq!(mapCounts.get("trigger"), Some(&5));
        assert_eq!(mapCounts.get("type"), Some(&5));
        assert_eq!(
            mapObjects.get(&(
                "constraint".to_string(),
                "comments.comments_topic_fkey".to_string()
            )),
            Some(
                &"[\"f\",false,false,true,\"FOREIGN KEY (topic) REFERENCES topics(id)\"]"
                    .to_string()
            )
        );
        assert_eq!(
            mapObjects.get(&("sequence".to_string(), "images_id_seq".to_string())),
            Some(&"[\"integer\",1,1,2147483647,1,1,false,\"images\",\"id\",\"i\"]".to_string())
        );
        assert_eq!(
            mapObjects.get(&("relation".to_string(), "topics".to_string())),
            Some(&"[\"r\",\"p\",false,false,\"heap\"]".to_string())
        );
        assert_eq!(
            mapObjects.get(&(
                "grant".to_string(),
                "function.topins().linuxweb.EXECUTE".to_string()
            )),
            Some(&"[true]".to_string())
        );
        assert_eq!(
            mapObjects.get(&("schema".to_string(), "public".to_string())),
            Some(&"[]".to_string())
        );
        assert_eq!(
            mapObjects.get(&("type".to_string(), "event_type".to_string())),
            Some(&"[\"e\",\"E\"]".to_string())
        );
        assert_eq!(
            mapObjects.get(&(
                "grant".to_string(),
                "type.event_type.linuxweb.USAGE".to_string()
            )),
            Some(&"[true]".to_string())
        );
    }

    #[test]
    fn schemaObjectComparisonFailsForMissingOrChangedRequiredObjects() {
        let mapExpected = TySchemaObjectMap::from([
            (
                ("constraint".to_string(), "topics.topics_pkey".to_string()),
                "[\"p\"]".to_string(),
            ),
            (
                ("default".to_string(), "topics.stat1".to_string()),
                "[\"0\"]".to_string(),
            ),
        ]);
        let mapActual = TySchemaObjectMap::from([(
            ("default".to_string(), "topics.stat1".to_string()),
            "[\"1\"]".to_string(),
        )]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert_eq!(stComparison.vecBlockingProblems.len(), 2);
        assert!(
            stComparison.vecBlockingProblems[0].contains("missing constraint topics.topics_pkey")
        );
        assert!(stComparison.vecBlockingProblems[1].contains("changed default topics.stat1"));
    }

    #[test]
    fn schemaObjectComparisonReportsPermittedAdditionalObjectsAsDrift() {
        let mapExpected = TySchemaObjectMap::from([(
            ("index".to_string(), "topics.topics_pkey".to_string()),
            "[true]".to_string(),
        )]);
        let mapActual = TySchemaObjectMap::from([
            (
                ("index".to_string(), "topics.topics_pkey".to_string()),
                "[true]".to_string(),
            ),
            (
                (
                    "index".to_string(),
                    "topics.operator_observability_idx".to_string(),
                ),
                "[false]".to_string(),
            ),
        ]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert!(stComparison.vecBlockingProblems.is_empty());
        assert_eq!(
            stComparison.vecAdvisoryDrift,
            ["additional index topics.operator_observability_idx"]
        );
    }

    #[test]
    fn schemaObjectExporterUsesEffectiveRuntimeGrantsAndOmitsPhysicalFlags() {
        assert!(S_SCHEMA_OBJECTS_QUERY.contains("has_table_privilege('linuxweb'"));
        assert!(S_SCHEMA_OBJECTS_QUERY.contains("has_sequence_privilege('linuxweb'"));
        assert!(S_SCHEMA_OBJECTS_QUERY.contains("has_function_privilege('linuxweb'"));
        assert!(S_SCHEMA_OBJECTS_QUERY.contains("jsonb_build_array(relation.relacl::text)"));
        assert!(!S_SCHEMA_OBJECTS_QUERY.contains("relreplident"));
        assert!(!S_SCHEMA_OBJECTS_QUERY.contains("indisclustered"));
        assert!(!S_SCHEMA_OBJECTS_QUERY.contains("indisreplident"));
    }

    #[test]
    fn directAclDriftIsAdvisoryWhenEffectiveGrantStillMatches() {
        let mapExpected = TySchemaObjectMap::from([
            (
                ("acl".to_string(), "table.topics".to_string()),
                "[\"{linuxweb=r/maxcom}\"]".to_string(),
            ),
            (
                (
                    "grant".to_string(),
                    "table.topics.linuxweb.SELECT".to_string(),
                ),
                "[true]".to_string(),
            ),
        ]);
        let mapActual = TySchemaObjectMap::from([
            (
                ("acl".to_string(), "table.topics".to_string()),
                "[\"{runtime_reader=r/maxcom}\"]".to_string(),
            ),
            (
                (
                    "grant".to_string(),
                    "table.topics.linuxweb.SELECT".to_string(),
                ),
                "[true]".to_string(),
            ),
        ]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert!(stComparison.vecBlockingProblems.is_empty());
        assert_eq!(stComparison.vecAdvisoryDrift.len(), 1);
        assert!(stComparison.vecAdvisoryDrift[0].contains("changed acl table.topics"));
    }

    #[test]
    fn additionalConstraintsAndTriggersAreBlockingButIndexesRemainAdvisory() {
        let mapExpected = TySchemaObjectMap::new();
        let mapActual = TySchemaObjectMap::from([
            (
                (
                    "constraint".to_string(),
                    "topics.operator_check".to_string(),
                ),
                "[\"c\"]".to_string(),
            ),
            (
                ("trigger".to_string(), "topics.operator_trigger".to_string()),
                "[\"O\"]".to_string(),
            ),
            (
                (
                    "index".to_string(),
                    "topics.operator_observability_idx".to_string(),
                ),
                "[false]".to_string(),
            ),
        ]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert_eq!(stComparison.vecBlockingProblems.len(), 2);
        assert!(
            stComparison.vecBlockingProblems[0]
                .contains("additional constraint topics.operator_check")
        );
        assert!(
            stComparison.vecBlockingProblems[1]
                .contains("additional trigger topics.operator_trigger")
        );
        assert_eq!(
            stComparison.vecAdvisoryDrift,
            ["additional index topics.operator_observability_idx"]
        );
    }

    #[test]
    fn schemaOwnerDifferencesAreAdvisoryButRequiredRuntimeGrantsAreBlocking() {
        let mapExpected = TySchemaObjectMap::from([
            (
                ("owner".to_string(), "table.topics".to_string()),
                "[\"maxcom\"]".to_string(),
            ),
            (
                (
                    "grant".to_string(),
                    "table.topics.linuxweb.SELECT".to_string(),
                ),
                "[true]".to_string(),
            ),
        ]);
        let mapActual = TySchemaObjectMap::from([(
            ("owner".to_string(), "table.topics".to_string()),
            "[\"migration_owner\"]".to_string(),
        )]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert_eq!(stComparison.vecBlockingProblems.len(), 1);
        assert!(stComparison.vecBlockingProblems[0].contains("missing grant"));
        assert_eq!(stComparison.vecAdvisoryDrift.len(), 1);
        assert!(stComparison.vecAdvisoryDrift[0].contains("changed owner"));
    }

    #[test]
    fn schemaRlsAndFunctionExecuteDifferencesAreBlocking() {
        let mapExpected = TySchemaObjectMap::from([
            (
                ("relation".to_string(), "topics".to_string()),
                "[\"r\",\"p\",false,false,\"heap\"]".to_string(),
            ),
            (
                (
                    "grant".to_string(),
                    "function.topins().linuxweb.EXECUTE".to_string(),
                ),
                "[true]".to_string(),
            ),
        ]);
        let mapActual = TySchemaObjectMap::from([(
            ("relation".to_string(), "topics".to_string()),
            "[\"r\",\"p\",true,false,\"heap\"]".to_string(),
        )]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert_eq!(stComparison.vecBlockingProblems.len(), 2);
        assert!(
            stComparison.vecBlockingProblems[0]
                .contains("missing grant function.topins().linuxweb.EXECUTE")
        );
        assert!(stComparison.vecBlockingProblems[1].contains("changed relation topics"));
    }

    #[test]
    fn schemaAndEnumUsageDifferencesAreBlocking() {
        let mapExpected = TySchemaObjectMap::from([
            (
                (
                    "grant".to_string(),
                    "schema.public.linuxweb.USAGE".to_string(),
                ),
                "[true]".to_string(),
            ),
            (
                (
                    "grant".to_string(),
                    "type.event_type.linuxweb.USAGE".to_string(),
                ),
                "[true]".to_string(),
            ),
        ]);
        let mapActual = TySchemaObjectMap::from([(
            (
                "grant".to_string(),
                "schema.public.linuxweb.USAGE".to_string(),
            ),
            "[false]".to_string(),
        )]);

        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);
        assert_eq!(stComparison.vecBlockingProblems.len(), 2);
        assert!(
            stComparison.vecBlockingProblems[0]
                .contains("changed grant schema.public.linuxweb.USAGE")
        );
        assert!(
            stComparison.vecBlockingProblems[1]
                .contains("missing grant type.event_type.linuxweb.USAGE")
        );
    }

    #[test]
    fn schemaObjectFingerprintIsDeterministicAndDefinitionSensitive() {
        let mapFirst = TySchemaObjectMap::from([
            (
                ("default".to_string(), "topics.stat1".to_string()),
                "[\"0\"]".to_string(),
            ),
            (
                ("sequence".to_string(), "s_msgid".to_string()),
                "[\"bigint\"]".to_string(),
            ),
        ]);
        let mapSameDifferentInsertionOrder = TySchemaObjectMap::from([
            (
                ("sequence".to_string(), "s_msgid".to_string()),
                "[\"bigint\"]".to_string(),
            ),
            (
                ("default".to_string(), "topics.stat1".to_string()),
                "[\"0\"]".to_string(),
            ),
        ]);
        let mut mapChanged = mapFirst.clone();
        mapChanged.insert(
            ("default".to_string(), "topics.stat1".to_string()),
            "[\"1\"]".to_string(),
        );

        assert_eq!(
            sSchemaObjectFingerprint(&mapFirst),
            sSchemaObjectFingerprint(&mapSameDifferentInsertionOrder)
        );
        assert_ne!(
            sSchemaObjectFingerprint(&mapFirst),
            sSchemaObjectFingerprint(&mapChanged)
        );
    }

    #[test]
    fn schemaObjectCatalogQueryIsReadOnlyAndBounded() {
        let sLower = S_SCHEMA_OBJECTS_QUERY.to_ascii_lowercase();
        assert!(sLower.contains("canonical_tables(table_name)"));
        assert!(sLower.contains("canonical_sequences(sequence_name)"));
        assert!(sLower.contains("canonical_functions(function_name)"));
        for sMutation in [" insert ", " update ", " delete ", " alter ", " drop "] {
            assert!(
                !sLower.contains(sMutation),
                "catalog query must not contain mutation token {sMutation:?}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected canonical Java/Liquibase PostgreSQL database"]
    async fn canonicalSchemaObjectContractMatchesRuntimeCatalog() {
        assert_eq!(
            std::env::var("LOR_SCHEMA_INTEGRATION_CONFIRM").as_deref(),
            Ok("read-only-canonical-contract"),
            "set LOR_SCHEMA_INTEGRATION_CONFIRM=read-only-canonical-contract"
        );
        let sDatabaseUrl = std::env::var("LOR_SCHEMA_INTEGRATION_DATABASE_URL")
            .expect("set LOR_SCHEMA_INTEGRATION_DATABASE_URL to a disposable canonical database");
        let oPool = oConnect(&sDatabaseUrl)
            .await
            .expect("canonical test database must be reachable");
        let mapExpected = mapExpectedSchemaObjects().expect("contract must parse");
        let mapActual = mapReadSchemaObjects(&oPool)
            .await
            .expect("catalog query must succeed through the runtime role");
        let stComparison = stCompareSchemaObjects(&mapExpected, &mapActual);

        assert!(
            stComparison.vecBlockingProblems.is_empty(),
            "blocking differences: {:?}",
            stComparison.vecBlockingProblems
        );
        assert!(
            stComparison.vecAdvisoryDrift.is_empty(),
            "fresh canonical bootstrap must have no drift: {:?}",
            stComparison.vecAdvisoryDrift
        );
        assert_eq!(
            sSchemaObjectFingerprint(&mapExpected),
            sSchemaObjectFingerprint(&mapActual)
        );
    }

    #[tokio::test]
    async fn database_connect_errors_do_not_disclose_credentials() {
        let sSecretUrl = "postgres://runtime:super-secret@127.0.0.1:1/lor";
        let stError = oConnect(sSecretUrl)
            .await
            .expect_err("closed local port must reject the connection");
        let sError = format!("{stError:#}");

        assert!(sError.contains("failed to connect to PostgreSQL"));
        assert!(!sError.contains("super-secret"));
        assert!(!sError.contains(sSecretUrl));
    }
}
