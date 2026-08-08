use anyhow::Context;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tracing::info;

pub type TyPgPool = PgPool;

const S_SCHEMA_CONTRACT: &str = include_str!("../../../compat/java-db/schema-contract.tsv");

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

pub async fn oConnect(sDatabaseUrl: &str) -> anyhow::Result<TyPgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(sDatabaseUrl)
        .await
        .with_context(|| format!("failed to connect to PostgreSQL: {sDatabaseUrl}"))
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

    info!(
        "validated current Java database structure without reading or changing the Liquibase ledger"
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
}
