//! Java-compatible scheduled maintenance and external publishing jobs.
//!
//! Every job takes a PostgreSQL advisory lock. This preserves the original
//! single-scheduler semantics when the Rust application runs with more than
//! one replica during or after migration.

use std::{future::Future, path::PathBuf, time::Duration};

use anyhow::Context;
use chrono::{Datelike, Timelike, Utc};
use chrono_tz::Europe::Moscow;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::{sync::watch, task::JoinHandle};

use crate::{application::exception_reporting::StExceptionReport, state::AppState};

const LOCK_STATS: i64 = 0x4c4f_5201;
const LOCK_GROUP_STATS: i64 = 0x4c4f_5202;
const LOCK_TAG_COUNTERS: i64 = 0x4c4f_5203;
const LOCK_OLD_EVENTS: i64 = 0x4c4f_5204;
const LOCK_SCORE: i64 = 0x4c4f_5205;
const LOCK_MAX_SCORE: i64 = 0x4c4f_5206;
const LOCK_LOW_SCORE: i64 = 0x4c4f_5207;
const LOCK_INACTIVE_USERS: i64 = 0x4c4f_5208;
const LOCK_TOR: i64 = 0x4c4f_5209;
const LOCK_EMAIL_DOMAINS: i64 = 0x4c4f_520a;
const LOCK_TELEGRAM: i64 = 0x4c4f_520b;
const LOCK_USERPICS: i64 = 0x4c4f_520c;
const LOCK_GALLERY_PREVIEWS: i64 = 0x4c4f_520d;

const HOUR: Duration = Duration::from_secs(60 * 60);
const FOUR_HOURS: Duration = Duration::from_secs(4 * 60 * 60);
const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);
const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);
const S_TOR_EXIT_LIST_URL: &str = "https://www.dan.me.uk/torlist/?exit";
const S_DISPOSABLE_DOMAINS_URL: &str =
    "https://disposable.github.io/disposable-email-domains/domains_mx.txt";

pub fn vecSpawn(stState: AppState, oShutdown: watch::Receiver<bool>) -> Vec<JoinHandle<()>> {
    let mut vecJobs = vec![
        stSpawnSearchQueue(stState.clone(), oShutdown.clone()),
        stSpawnAdvCounters(stState.clone(), oShutdown.clone()),
    ];
    if !stState.config.enable_background_jobs {
        tracing::info!("maintenance and external background jobs disabled by configuration");
        return vecJobs;
    }

    vecJobs.extend([
        stSpawnFixed(
            "statistics",
            FIVE_MINUTES,
            TEN_MINUTES,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateStatistics(&stState.pool).await },
        ),
        stSpawnFixed(
            "group statistics",
            FIVE_MINUTES,
            HOUR,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateGroupStatistics(&stState.pool).await },
        ),
        stSpawnFixed(
            "tag counters",
            FIVE_MINUTES,
            HOUR,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateTagCounters(&stState.pool).await },
        ),
        stSpawnFixed(
            "old events",
            FIVE_MINUTES,
            HOUR,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vCleanupOldEvents(&stState.pool).await },
        ),
        stSpawnFixed(
            "old gallery previews",
            FIVE_MINUTES,
            HOUR,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vCleanupGalleryPreviews(&stState).await },
        ),
        stSpawnFixed(
            "disposable email domains",
            Duration::from_secs(60),
            FOUR_HOURS,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateDisposableDomains(&stState).await },
        ),
        stSpawnFixed(
            "TOR exit nodes",
            Duration::from_secs(30 * 60),
            HOUR,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateTorExitNodes(&stState).await },
        ),
        stSpawnFixed(
            "Telegram publisher",
            Duration::from_secs(60),
            FIVE_MINUTES,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateTelegram(&stState).await },
        ),
        stSpawnHourly(
            "maximum score",
            15,
            1,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateMaxScore(&stState.pool).await },
        ),
        stSpawnHourly(
            "low-score blocking",
            1,
            0,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vBlockLowScoreUsers(&stState.pool).await },
        ),
        stSpawnHourly(
            "inactive accounts",
            30,
            0,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vDeleteInactiveAccounts(&stState.pool).await },
        ),
        stSpawnDaily(
            "score",
            1,
            0,
            1,
            stState.clone(),
            oShutdown.clone(),
            |stState| async move {
                // Spring's `*/2` in the day-of-month field fires on 1,3,5,...
                if Utc::now().with_timezone(&Moscow).day() % 2 == 1 {
                    vUpdateScore(&stState.pool).await
                } else {
                    Ok(())
                }
            },
        ),
        stSpawnDaily(
            "old userpics",
            4,
            30,
            0,
            stState.clone(),
            oShutdown,
            |stState| async move { vCleanupOldUserpics(&stState).await },
        ),
    ]);
    vecJobs
}

fn stSpawnSearchQueue(stState: AppState, mut oShutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        if bWaitOrShutdown(Duration::from_secs(1), &mut oShutdown).await {
            return;
        }
        loop {
            if let Err(stError) = crate::search_index::vDrainQueue(&stState).await {
                // The Java search listener is a broker consumer, not a
                // Spring `@Scheduled` method governed by TaskScheduler's
                // ErrorHandler. Keep its existing log-only behavior here.
                tracing::error!(job = "search queue", error = %stError, "background job failed");
            }
            if bWaitOrShutdown(Duration::from_secs(5), &mut oShutdown).await {
                return;
            }
        }
    })
}

fn stSpawnAdvCounters(stState: AppState, mut oShutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let bShutdown = bWaitOrShutdown(Duration::from_secs(60), &mut oShutdown).await;
            if let Err(stError) = vFlushAdvCounters(&stState).await {
                tracing::error!(job = "advertisement counters", error = %stError, "background job failed");
            }
            if bShutdown {
                return;
            }
        }
    })
}

async fn vFlushAdvCounters(stState: &AppState) -> anyhow::Result<()> {
    let mapBatch = stState.adv_counter.mapTake();
    if mapBatch.is_empty() {
        return Ok(());
    }

    let stResult = async {
        let mut stTransaction = stState.pool.begin().await?;
        for (sPath, iIncrement) in &mapBatch {
            sqlx::query(
                r#"INSERT INTO adv_counts(path,day,counter)
                   VALUES($1,CURRENT_DATE,$2)
                   ON CONFLICT(path,day) DO UPDATE
                   SET counter=adv_counts.counter+excluded.counter"#,
            )
            .bind(sPath)
            .bind(iIncrement)
            .execute(&mut *stTransaction)
            .await?;
        }
        stTransaction.commit().await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;

    if let Err(stError) = stResult {
        stState.adv_counter.vRestore(mapBatch);
        return Err(stError.into());
    }
    Ok(())
}

fn stSpawnFixed<F, Fut>(
    sName: &'static str,
    stInitialDelay: Duration,
    stDelay: Duration,
    stState: AppState,
    mut oShutdown: watch::Receiver<bool>,
    fRun: F,
) -> JoinHandle<()>
where
    F: Fn(AppState) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        if bWaitOrShutdown(stInitialDelay, &mut oShutdown).await {
            return;
        }
        loop {
            if let Err(stError) = fRun(stState.clone()).await {
                vReportScheduledFailure(&stState, sName, &stError);
            }
            if bWaitOrShutdown(stDelay, &mut oShutdown).await {
                return;
            }
        }
    })
}

fn stSpawnHourly<F, Fut>(
    sName: &'static str,
    iMinute: u32,
    iSecond: u32,
    stState: AppState,
    mut oShutdown: watch::Receiver<bool>,
    fRun: F,
) -> JoinHandle<()>
where
    F: Fn(AppState) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            if bWaitOrShutdown(stUntilNextHour(iMinute, iSecond), &mut oShutdown).await {
                return;
            }
            if let Err(stError) = fRun(stState.clone()).await {
                vReportScheduledFailure(&stState, sName, &stError);
            }
        }
    })
}

fn stSpawnDaily<F, Fut>(
    sName: &'static str,
    iHour: u32,
    iMinute: u32,
    iSecond: u32,
    stState: AppState,
    mut oShutdown: watch::Receiver<bool>,
    fRun: F,
) -> JoinHandle<()>
where
    F: Fn(AppState) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            if bWaitOrShutdown(stUntilNextDay(iHour, iMinute, iSecond), &mut oShutdown).await {
                return;
            }
            if let Err(stError) = fRun(stState.clone()).await {
                vReportScheduledFailure(&stState, sName, &stError);
            }
        }
    })
}

fn vReportScheduledFailure(stState: &AppState, sName: &str, stError: &anyhow::Error) {
    tracing::error!(job = sName, error = %stError, "background job failed");
    stState.exception_reporter.vReport(stScheduledFailureReport(
        sName,
        stError,
        stState.config.telegram_token.as_deref(),
    ));
}

fn stScheduledFailureReport(
    sName: &str,
    stError: &anyhow::Error,
    optTelegramToken: Option<&str>,
) -> StExceptionReport {
    let mut sError = format!("{stError:#}");
    if let Some(sToken) = optTelegramToken.filter(|sToken| !sToken.is_empty()) {
        sError = sError.replace(sToken, "[REDACTED]");
    }
    StExceptionReport {
        // `anyhow` deliberately erases the concrete exception class which the
        // Java scheduler used for rate grouping. Keep jobs distinct so one
        // failing task cannot suppress the first report from another task.
        sType: format!("Periodic task: {sName}"),
        sBody: format!("Periodic task failed\n\nJob: {sName}\n{sError}"),
    }
}

async fn bWaitOrShutdown(stDelay: Duration, oShutdown: &mut watch::Receiver<bool>) -> bool {
    if *oShutdown.borrow() {
        return true;
    }
    tokio::select! {
        () = tokio::time::sleep(stDelay) => false,
        stChanged = oShutdown.changed() => stChanged.is_err() || *oShutdown.borrow(),
    }
}

fn stUntilNextHour(iMinute: u32, iSecond: u32) -> Duration {
    let dtNow = Utc::now().with_timezone(&Moscow);
    let mut dtNext = dtNow
        .with_minute(iMinute)
        .and_then(|dt| dt.with_second(iSecond))
        .and_then(|dt| dt.with_nanosecond(0))
        .expect("valid scheduler minute and second");
    if dtNext <= dtNow {
        dtNext += chrono::Duration::hours(1);
    }
    (dtNext - dtNow).to_std().unwrap_or(Duration::from_secs(1))
}

fn stUntilNextDay(iHour: u32, iMinute: u32, iSecond: u32) -> Duration {
    let dtNow = Utc::now().with_timezone(&Moscow);
    let mut dtNext = dtNow
        .with_hour(iHour)
        .and_then(|dt| dt.with_minute(iMinute))
        .and_then(|dt| dt.with_second(iSecond))
        .and_then(|dt| dt.with_nanosecond(0))
        .expect("valid scheduler time");
    if dtNext <= dtNow {
        dtNext += chrono::Duration::days(1);
    }
    (dtNext - dtNow).to_std().unwrap_or(Duration::from_secs(1))
}

async fn bLock(stTransaction: &mut Transaction<'_, Postgres>, iLock: i64) -> anyhow::Result<bool> {
    sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(iLock)
        .fetch_one(&mut **stTransaction)
        .await
        .context("acquiring background-job advisory lock")
}

async fn vUpdateStatistics(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_STATS).await? {
        return Ok(());
    }
    sqlx::query("SELECT stat_update()")
        .execute(&mut *stTransaction)
        .await?;
    sqlx::query("SELECT update_monthly_stats()")
        .execute(&mut *stTransaction)
        .await?;
    sqlx::query(
        r#"UPDATE topics SET open_warnings=(
             SELECT count(DISTINCT mw.author) FROM message_warnings mw
             WHERE mw.topic=topics.id AND mw.comment IS NULL AND mw.closed_by IS NULL
               AND mw.warning_type='rule' AND mw.postdate>CURRENT_TIMESTAMP-'12 hours'::interval
               AND mw.author IN (SELECT id FROM users WHERE score>100))
           WHERE open_warnings>0"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    stTransaction.commit().await?;
    Ok(())
}

async fn vUpdateGroupStatistics(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if bLock(&mut stTransaction, LOCK_GROUP_STATS).await? {
        sqlx::query("SELECT stat_update2()")
            .execute(&mut *stTransaction)
            .await?;
        stTransaction.commit().await?;
    }
    Ok(())
}

async fn vUpdateTagCounters(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_TAG_COUNTERS).await? {
        return Ok(());
    }
    sqlx::query(
        r#"UPDATE tags_values SET counter=(
             SELECT count(*) FROM tags JOIN topics ON tags.msgid=topics.id
             JOIN groups ON topics.groupid=groups.id JOIN sections ON sections.id=groups.section
             WHERE tags.tagid=tags_values.id AND NOT deleted
               AND (topics.moderate OR NOT sections.moderate))"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    let stDeleted = sqlx::query(
        "DELETE FROM user_tags WHERE NOT EXISTS (SELECT 1 FROM tags JOIN topics ON topics.id=tags.msgid WHERE tagid=user_tags.tag_id AND NOT deleted)",
    )
    .execute(&mut *stTransaction)
    .await?
    .rows_affected();
    stTransaction.commit().await?;
    if stDeleted > 0 {
        tracing::info!(count = stDeleted, "deleted empty favorite tags");
    }
    Ok(())
}

async fn vCleanupOldEvents(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_OLD_EVENTS).await? {
        return Ok(());
    }
    let vecUsers = sqlx::query_scalar::<_, i32>(
        "SELECT userid FROM user_events GROUP BY userid HAVING count(id)>4000 ORDER BY count(id) DESC LIMIT 20",
    )
    .fetch_all(&mut *stTransaction)
    .await?;
    for iUserId in &vecUsers {
        sqlx::query(
            "DELETE FROM user_events WHERE id IN (SELECT id FROM user_events WHERE userid=$1 ORDER BY event_date DESC OFFSET 4000)",
        )
        .bind(iUserId)
        .execute(&mut *stTransaction)
        .await?;
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events WHERE unread AND userid=users.id) WHERE id=$1")
            .bind(iUserId)
            .execute(&mut *stTransaction)
            .await?;
    }
    let stDeleted = sqlx::query(
        r#"DELETE FROM user_events WHERE event_date<CURRENT_TIMESTAMP-'2 year'::interval
           AND userid IN (SELECT id FROM users WHERE blocked AND lastlogin<CURRENT_TIMESTAMP-'2 year'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?
    .rows_affected();
    sqlx::query(
        r#"UPDATE users SET unread_events=(SELECT count(*) FROM user_events WHERE unread AND userid=users.id)
           WHERE unread_events!=0 AND blocked AND lastlogin<CURRENT_TIMESTAMP-'2 year'::interval"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    stTransaction.commit().await?;
    tracing::info!(
        users = vecUsers.len(),
        abandoned = stDeleted,
        "old events cleanup complete"
    );
    Ok(())
}

async fn vUpdateScore(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_SCORE).await? {
        return Ok(());
    }
    sqlx::query(
        r#"UPDATE users SET score=score+1 WHERE id IN (
             SELECT DISTINCT comments.userid FROM comments,topics
             WHERE comments.postdate>CURRENT_TIMESTAMP-'2 days'::interval
               AND topics.id=comments.topic AND topics.groupid NOT IN (8404,4068,9326,19405)
               AND NOT comments.deleted AND NOT topics.deleted AND NOT topics.notop)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    sqlx::query("UPDATE users SET max_score=score WHERE score>max_score")
        .execute(&mut *stTransaction)
        .await?;
    stTransaction.commit().await?;
    Ok(())
}

async fn vUpdateMaxScore(oPool: &PgPool) -> anyhow::Result<()> {
    vExecuteLocked(
        oPool,
        LOCK_MAX_SCORE,
        "UPDATE users SET max_score=score WHERE score>max_score",
    )
    .await
}

async fn vBlockLowScoreUsers(oPool: &PgPool) -> anyhow::Result<()> {
    vExecuteLocked(
        oPool,
        LOCK_LOW_SCORE,
        "UPDATE users SET blocked=true WHERE score < -50 AND nick!='anonymous' AND max_score<150 AND NOT blocked",
    )
    .await
}

async fn vDeleteInactiveAccounts(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_INACTIVE_USERS).await? {
        return Ok(());
    }
    let vecRegular = vecDeletableInactiveUsers(&mut stTransaction, true).await?;
    let iRegular = vecRegular.len();
    vDeleteInactiveUserBatch(&mut stTransaction, &vecRegular).await?;
    let vecBlocked = vecDeletableInactiveUsers(&mut stTransaction, false).await?;
    let iBlocked = vecBlocked.len();
    vDeleteInactiveUserBatch(&mut stTransaction, &vecBlocked).await?;
    stTransaction.commit().await?;
    tracing::info!(
        regular = iRegular,
        blocked = iBlocked,
        "deleted inactive accounts"
    );
    Ok(())
}

async fn vecDeletableInactiveUsers(
    stTransaction: &mut Transaction<'_, Postgres>,
    bRegularWindow: bool,
) -> anyhow::Result<Vec<i32>> {
    // Registration creates user_settings before activation, while the Java
    // cleanup SQL deletes users directly even though that FK has no cascade.
    // Historic databases also contain a few unactivated accounts referenced
    // by content. Select only accounts whose owned rows can be removed without
    // deleting forum content or making the entire hourly transaction fail.
    Ok(sqlx::query_scalar(
        r#"SELECT u.id FROM users u
           WHERE NOT u.activated
             AND (($1 AND NOT u.blocked AND u.regdate<CURRENT_TIMESTAMP-'12 hours'::interval)
               OR (NOT $1 AND u.regdate<CURRENT_TIMESTAMP-'30 days'::interval))
             AND NOT EXISTS (SELECT 1 FROM topics t WHERE t.userid=u.id OR t.commitby=u.id)
             AND NOT EXISTS (SELECT 1 FROM comments c WHERE c.userid=u.id OR c.editor_id=u.id)
             AND NOT EXISTS (SELECT 1 FROM ban_info b WHERE b.ban_by=u.id)
             AND NOT EXISTS (SELECT 1 FROM del_info d WHERE d.delby=u.id)
             AND NOT EXISTS (SELECT 1 FROM edit_info e WHERE e.editor=u.id)
             AND NOT EXISTS (SELECT 1 FROM ignore_list i WHERE i.userid=u.id OR i.ignored=u.id)
             AND NOT EXISTS (SELECT 1 FROM memories m WHERE m.userid=u.id)
             AND NOT EXISTS (SELECT 1 FROM vote_users v WHERE v.userid=u.id)
             AND NOT EXISTS (SELECT 1 FROM user_tags t WHERE t.user_id=u.id)
             AND NOT EXISTS (SELECT 1 FROM user_remarks r WHERE r.user_id=u.id)
             AND NOT EXISTS (SELECT 1 FROM user_invites i WHERE i.owner=u.id OR i.invited_user=u.id)
             AND NOT EXISTS (SELECT 1 FROM reactions_log r WHERE r.origin_user=u.id)
             AND NOT EXISTS (SELECT 1 FROM message_warnings w WHERE w.author=u.id OR w.closed_by=u.id)
             AND NOT EXISTS (SELECT 1 FROM user_events e WHERE e.origin_user=u.id AND e.userid<>u.id)
             AND NOT EXISTS (SELECT 1 FROM users x WHERE x.frozen_by=u.id)"#,
    )
    .bind(bRegularWindow)
    .fetch_all(&mut **stTransaction)
    .await?)
}

async fn vDeleteInactiveUserBatch(
    stTransaction: &mut Transaction<'_, Postgres>,
    vecUserIds: &[i32],
) -> anyhow::Result<()> {
    if vecUserIds.is_empty() {
        return Ok(());
    }
    for sSql in [
        "DELETE FROM user_events WHERE userid=ANY($1)",
        "DELETE FROM topic_users_notified WHERE userid=ANY($1)",
        "DELETE FROM ban_info WHERE userid=ANY($1)",
        "DELETE FROM user_settings WHERE id=ANY($1)",
        "DELETE FROM users WHERE id=ANY($1)",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sSql))
            .bind(vecUserIds)
            .execute(&mut **stTransaction)
            .await?;
    }
    Ok(())
}

async fn vExecuteLocked(oPool: &PgPool, iLock: i64, sSql: &'static str) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if bLock(&mut stTransaction, iLock).await? {
        sqlx::query(sSql).execute(&mut *stTransaction).await?;
        stTransaction.commit().await?;
    }
    Ok(())
}

async fn vUpdateTorExitNodes(stState: &AppState) -> anyhow::Result<()> {
    let sBody = sFetchExternalList(&stState.http, S_TOR_EXIT_LIST_URL).await?;
    let mut oConnection = stState.pool.acquire().await?;
    let bAcquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(LOCK_TOR)
        .fetch_one(&mut *oConnection)
        .await?;
    if !bAcquired {
        return Ok(());
    }
    let stResult = vStoreTorExitNodes(&sBody, &mut oConnection).await;
    let _: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_TOR)
        .fetch_one(&mut *oConnection)
        .await?;
    stResult
}

async fn vStoreTorExitNodes(
    sBody: &str,
    oConnection: &mut sqlx::pool::PoolConnection<Postgres>,
) -> anyhow::Result<()> {
    // Java calls `IpBlockDao.blockIP` once per `linesIterator` item. Each
    // `springDB.run` commits independently, so an invalid later line leaves
    // earlier IPs committed. Keep that failure/commit contract rather than
    // wrapping the whole remote list in one Rust transaction.
    for sIp in vecJavaLines(sBody) {
        sqlx::query(
            r#"INSERT INTO b_ips(ip,mod_id,date,reason,ban_date,allow_posting,captcha_required)
               VALUES($1::inet,0,CURRENT_TIMESTAMP,'TOR Exit Node',CURRENT_TIMESTAMP+'1 month'::interval,true,false)
               ON CONFLICT(ip) DO UPDATE SET mod_id=EXCLUDED.mod_id,date=CURRENT_TIMESTAMP,
               reason=EXCLUDED.reason,ban_date=EXCLUDED.ban_date,
               allow_posting=EXCLUDED.allow_posting,captcha_required=EXCLUDED.captcha_required"#,
        )
        .bind(sIp)
        .execute(&mut **oConnection)
        .await?;
    }
    Ok(())
}

async fn vUpdateDisposableDomains(stState: &AppState) -> anyhow::Result<()> {
    let sBody = sFetchExternalList(&stState.http, S_DISPOSABLE_DOMAINS_URL).await?;
    let mut stTransaction = stState.pool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_EMAIL_DOMAINS).await? {
        return Ok(());
    }
    // Java passes `body.linesIterator.toVector` unchanged into one
    // `batchByName` transaction. Do not invent trimming/comment filtering.
    for sDomain in vecJavaLines(&sBody) {
        sqlx::query(
            r#"INSERT INTO email_domains_block(domain,block_until,auto,moderator_id,blocked_at)
               VALUES($1,CURRENT_TIMESTAMP+'7 days'::interval,true,NULL,CURRENT_TIMESTAMP)
               ON CONFLICT(domain) DO UPDATE SET block_until=EXCLUDED.block_until,
               blocked_at=EXCLUDED.blocked_at,auto=true WHERE email_domains_block.auto"#,
        )
        .bind(sDomain)
        .execute(&mut *stTransaction)
        .await?;
    }
    stTransaction.commit().await?;
    Ok(())
}

async fn sFetchExternalList(cHttp: &reqwest::Client, sUrl: &str) -> anyhow::Result<String> {
    Ok(cHttp
        .get(sUrl)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

fn vecJavaLines(sBody: &str) -> Vec<&str> {
    sBody.lines().collect()
}

fn sTelegramPostText(sStoredTitle: &str, optTags: Option<&str>, sLink: &str) -> String {
    // `TelegramPostsDao.hotTopic` materializes a Java `Topic`, whose
    // `fromResultSet` applies `StringUtil.makeTitle`; TelegramPoster then calls
    // `getTitleUnescaped` (one HTML4 entity layer). Reuse that exact title
    // representation pipeline instead of publishing the raw database value.
    let sTitle = crate::domain::title::sMakeTitlePlainForDisplay(sStoredTitle);
    let sTags = optTags
        .filter(|sTags| !sTags.is_empty())
        .map(|sTags| {
            sTags
                .split(',')
                .map(|sTag| format!("#{}", sTag.replace(' ', "")))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    format!("{sTitle} {sTags}\n\n{sLink}")
}

async fn vUpdateTelegram(stState: &AppState) -> anyhow::Result<()> {
    let Some(sToken) = stState.config.telegram_token.as_deref() else {
        return Ok(());
    };
    let mut oConnection = stState.pool.acquire().await?;
    let bAcquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(LOCK_TELEGRAM)
        .fetch_one(&mut *oConnection)
        .await?;
    if !bAcquired {
        return Ok(());
    }
    let stResult = vUpdateTelegramLocked(stState, sToken, &mut oConnection).await;
    let _: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_TELEGRAM)
        .fetch_one(&mut *oConnection)
        .await?;
    stResult
}

async fn vUpdateTelegramLocked(
    stState: &AppState,
    sToken: &str,
    oConnection: &mut sqlx::pool::PoolConnection<Postgres>,
) -> anyhow::Result<()> {
    type TyHotTopic = (i32, String, String, String, Option<String>);
    let optTopic: Option<TyHotTopic> = sqlx::query_as(
        r#"SELECT t.id,t.title,g.urlname,
           CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
             WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
           string_agg(tv.value,',' ORDER BY tv.value)
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE t.id IN (
             SELECT c.topic FROM comments c JOIN users u ON c.userid=u.id JOIN topics ti ON ti.id=c.topic
             WHERE c.postdate>CURRENT_TIMESTAMP-'5 hour'::interval AND u.score>=100 AND ti.groupid!=4068
               AND ti.open_warnings<=2 AND ti.id NOT IN (SELECT topic_id FROM telegram_posts)
               AND NOT ti.deleted AND NOT c.deleted AND NOT ti.notop AND NOT ti.draft
               AND ti.postscore IS DISTINCT FROM 10002
             GROUP BY c.topic HAVING count(DISTINCT c.userid)>=15
             ORDER BY count(DISTINCT c.userid) DESC LIMIT 1)
           GROUP BY t.id,t.title,g.urlname,s.id,s.name"#,
    )
    .fetch_optional(&mut **oConnection)
    .await?;
    if let Some((iTopicId, sTitle, sGroup, sSection, optTags)) = optTopic {
        let sLink = format!(
            "{}/{sSection}/{sGroup}/{iTopicId}",
            stState.config.public_url.trim_end_matches('/')
        );
        let sText = sTelegramPostText(&sTitle, optTags.as_deref(), &sLink);
        let sUrl = format!("https://api.telegram.org/bot{sToken}/sendMessage");
        let stJson: Value = stTelegramGet(
            stState,
            &sUrl,
            &[("chat_id", "@best_of_lor"), ("text", sText.as_str())],
        )
        .await?
        .json()
        .await
        .map_err(|stError| {
            anyhow::anyhow!(sTelegramHttpError("Telegram JSON decode failed", &stError))
        })?;
        let iTelegramId = stJson
            .pointer("/result/message_id")
            .and_then(Value::as_i64)
            .context("Telegram response has no result.message_id")?;
        sqlx::query("INSERT INTO telegram_posts(topic_id,telegram_id) VALUES($1,$2)")
            .bind(iTopicId)
            .bind(iTelegramId as i32)
            .execute(&mut **oConnection)
            .await?;
        return Ok(());
    }

    let optTelegramId: Option<i32> = sqlx::query_scalar(
        r#"SELECT tp.telegram_id FROM telegram_posts tp JOIN topics t ON tp.topic_id=t.id
           WHERE tp.postdate>CURRENT_TIMESTAMP-'47 hours'::interval
             AND (t.deleted OR t.notop OR t.open_warnings>2 OR t.postscore IS NOT DISTINCT FROM 10002)
           LIMIT 1"#,
    )
    .fetch_optional(&mut **oConnection)
    .await?;
    if let Some(iTelegramId) = optTelegramId {
        let sTelegramId = iTelegramId.to_string();
        let sUrl = format!("https://api.telegram.org/bot{sToken}/deleteMessage");
        stTelegramGet(
            stState,
            &sUrl,
            &[
                ("chat_id", "@best_of_lor"),
                ("message_id", sTelegramId.as_str()),
            ],
        )
        .await?;
        sqlx::query("DELETE FROM telegram_posts WHERE telegram_id=$1")
            .bind(iTelegramId)
            .execute(&mut **oConnection)
            .await?;
    }
    Ok(())
}

async fn stTelegramGet(
    stState: &AppState,
    sUrl: &str,
    vecParameters: &[(&str, &str)],
) -> anyhow::Result<reqwest::Response> {
    stTelegramRequest(
        &stState.http,
        stState.proxy_http.as_ref(),
        sUrl,
        vecParameters,
    )
    .await
}

async fn stTelegramRequest(
    cDirect: &reqwest::Client,
    optProxy: Option<&reqwest::Client>,
    sUrl: &str,
    vecParameters: &[(&str, &str)],
) -> anyhow::Result<reqwest::Response> {
    match cDirect.get(sUrl).query(vecParameters).send().await {
        Ok(stResponse) if stResponse.status().is_success() => Ok(stResponse),
        Ok(stResponse) => {
            tracing::warn!(status = %stResponse.status(), "direct Telegram request failed; trying fallback proxy");
            let cProxy = optProxy.context("Telegram fallback proxy is not configured")?;
            let stFallback =
                cProxy
                    .get(sUrl)
                    .query(vecParameters)
                    .send()
                    .await
                    .map_err(|stError| {
                        anyhow::anyhow!(sTelegramHttpError(
                            "Telegram fallback request failed",
                            &stError,
                        ))
                    })?;
            if !stFallback.status().is_success() {
                anyhow::bail!("Telegram fallback returned HTTP {}", stFallback.status());
            }
            Ok(stFallback)
        }
        Err(stError) => {
            tracing::warn!(
                error_kind = sReqwestErrorKind(&stError),
                "direct Telegram request failed; trying fallback proxy"
            );
            let cProxy = optProxy.context("Telegram fallback proxy is not configured")?;
            let stFallback =
                cProxy
                    .get(sUrl)
                    .query(vecParameters)
                    .send()
                    .await
                    .map_err(|stError| {
                        anyhow::anyhow!(sTelegramHttpError(
                            "Telegram fallback request failed",
                            &stError,
                        ))
                    })?;
            if !stFallback.status().is_success() {
                anyhow::bail!("Telegram fallback returned HTTP {}", stFallback.status());
            }
            Ok(stFallback)
        }
    }
}

fn sReqwestErrorKind(stError: &reqwest::Error) -> &'static str {
    if stError.is_timeout() {
        "timeout"
    } else if stError.is_connect() {
        "connect"
    } else if stError.is_decode() {
        "decode"
    } else if stError.is_body() {
        "body"
    } else if stError.is_request() {
        "request"
    } else if stError.is_status() {
        "status"
    } else {
        "unknown"
    }
}

fn sTelegramHttpError(sOperation: &str, stError: &reqwest::Error) -> String {
    // reqwest's Display/source chain may contain the complete request URL.
    // Telegram embeds the bot token in that URL, so never propagate the
    // original error into anyhow/tracing. Java's TelegramHttpFailedException
    // deliberately drops the SttpClientException cause for the same reason.
    format!("{sOperation} ({})", sReqwestErrorKind(stError))
}

async fn vCleanupOldUserpics(stState: &AppState) -> anyhow::Result<()> {
    let mut stTransaction = stState.pool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_USERPICS).await? {
        return Ok(());
    }
    let vecActive =
        sqlx::query_scalar::<_, String>("SELECT photo FROM users WHERE photo IS NOT NULL")
            .fetch_all(&mut *stTransaction)
            .await?;
    let stActive: std::collections::HashSet<_> = vecActive.into_iter().collect();
    let stDirectory = PathBuf::from(&stState.config.upload_dir).join("photos");
    if !stDirectory.is_dir() {
        tracing::warn!(path = %stDirectory.display(), "photos directory does not exist");
        stTransaction.commit().await?;
        return Ok(());
    }
    let dtRaceGuard = std::time::SystemTime::now() - Duration::from_secs(60 * 60);
    let stPattern = regex::Regex::new(r"^\d+(?::-?\d+)?\.\w+$")?;
    let mut vecCandidates = Vec::new();
    for stEntry in std::fs::read_dir(&stDirectory)
        .with_context(|| format!("reading {}", stDirectory.display()))?
    {
        let stPath = stEntry?.path();
        let Some(sName) = stPath.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !stPath.is_file() {
            continue;
        }
        if !stPattern.is_match(sName) {
            tracing::warn!(userpic = sName, "unexpected file in photos directory");
            continue;
        }
        if stActive.contains(sName) {
            continue;
        }
        let dtModified = match stPath
            .metadata()
            .and_then(|stMetadata| stMetadata.modified())
        {
            Ok(dtModified) => dtModified,
            Err(stError) => {
                tracing::warn!(userpic = sName, error = %stError, "cannot read userpic mtime");
                continue;
            }
        };
        if dtModified < dtRaceGuard {
            vecCandidates.push((sName.to_owned(), stPath));
        }
    }
    for stBatch in vecCandidates.chunks(500) {
        let vecNames: Vec<_> = stBatch.iter().map(|(sName, _)| sName.clone()).collect();
        let vecRecent = sqlx::query_scalar::<_, String>(
            r#"SELECT DISTINCT name FROM (
                 SELECT info->'old_userpic' AS name FROM user_log
                 WHERE action IN ('set_userpic'::user_log_action,'reset_userpic'::user_log_action)
                   AND info->'old_userpic'=ANY($1)
                   AND action_date>=CURRENT_TIMESTAMP-'1095 days'::interval
                 UNION ALL SELECT info->'new_userpic' FROM user_log
                 WHERE action='set_userpic'::user_log_action AND info->'new_userpic'=ANY($1)
                   AND action_date>=CURRENT_TIMESTAMP-'1095 days'::interval) q"#,
        )
        .bind(&vecNames)
        .fetch_all(&mut *stTransaction)
        .await?;
        let stRecent: std::collections::HashSet<_> = vecRecent.into_iter().collect();
        for (sName, stPath) in stBatch {
            if !stRecent.contains(sName) {
                if stState.config.clean_old_userpics {
                    match std::fs::remove_file(stPath) {
                        Ok(()) => tracing::info!(userpic = sName, "deleted old userpic"),
                        Err(stError) if stError.kind() == std::io::ErrorKind::NotFound => {
                            tracing::info!(userpic = sName, "old userpic already removed");
                        }
                        Err(stError) => {
                            tracing::warn!(userpic = sName, error = %stError, "failed to delete old userpic");
                        }
                    }
                } else {
                    tracing::info!(userpic = sName, "old userpic candidate (dry run)");
                }
            }
        }
    }
    stTransaction.commit().await?;
    Ok(())
}

async fn vCleanupGalleryPreviews(stState: &AppState) -> anyhow::Result<()> {
    let mut stTransaction = stState.pool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_GALLERY_PREVIEWS).await? {
        return Ok(());
    }
    let stDirectory = PathBuf::from(&stState.config.upload_dir).join("gallery/preview");
    if !stDirectory.is_dir() {
        stTransaction.commit().await?;
        return Ok(());
    }
    let stThreshold = std::time::SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
    for stEntry in std::fs::read_dir(&stDirectory)? {
        let stPath = stEntry?.path();
        if stPath.is_file() && stPath.metadata()?.modified()? < stThreshold {
            std::fs::remove_file(&stPath)?;
            tracing::info!(path = %stPath.display(), "deleted old gallery preview");
        }
    }
    stTransaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{
        sFetchExternalList, sTelegramHttpError, sTelegramPostText, stScheduledFailureReport,
        stTelegramRequest, stUntilNextDay, stUntilNextHour, vecJavaLines,
    };

    #[test]
    fn external_lists_preserve_java_lines_iterator_values() {
        assert_eq!(
            vecJavaLines(" one\n\n# note\r\n two \n"),
            vec![" one", "", "# note", " two "]
        );
    }

    #[test]
    fn scheduler_delays_are_bounded() {
        assert!(stUntilNextHour(15, 1).as_secs() <= 3600);
        assert!(stUntilNextDay(4, 30, 0).as_secs() <= 24 * 3600);
    }

    #[test]
    fn every_spring_style_scheduler_loop_reports_and_redacts_failures() {
        let stError =
            anyhow::anyhow!("request https://api.telegram.org/botSUPER_SECRET/sendMessage failed")
                .context("publisher adapter");
        let stReport =
            stScheduledFailureReport("Telegram publisher", &stError, Some("SUPER_SECRET"));
        assert_eq!(stReport.sType, "Periodic task: Telegram publisher");
        assert!(stReport.sBody.contains("publisher adapter"));
        assert!(stReport.sBody.contains("Periodic task failed"));
        assert!(!stReport.sBody.contains("SUPER_SECRET"));
        assert!(stReport.sBody.contains("[REDACTED]"));

        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        assert_eq!(
            sProduction
                .matches("vReportScheduledFailure(&stState, sName, &stError)")
                .count(),
            3,
            "fixed-delay, hourly and daily Spring-style loops must all report"
        );
        assert!(
            !sProduction
                .split("fn stSpawnSearchQueue")
                .nth(1)
                .and_then(|sSource| sSource.split("fn stSpawnAdvCounters").next())
                .unwrap_or_default()
                .contains("vReportScheduledFailure")
        );
        assert!(
            !sProduction
                .split("fn stSpawnAdvCounters")
                .nth(1)
                .and_then(|sSource| sSource.split("fn vFlushAdvCounters").next())
                .unwrap_or_default()
                .contains("vReportScheduledFailure")
        );
    }

    #[test]
    fn telegram_text_matches_java_topic_title_and_tag_pipeline() {
        assert_eq!(
            sTelegramPostText(
                "&quot;LOR&quot; &amp; Rust &amp;lt; &amp;apos; &bogus; &amp",
                Some("два слова,rust"),
                "https://example.test/forum/linux/42",
            ),
            "«LOR» & Rust &lt; &apos; &bogus; &amp #дваслова #rust\n\nhttps://example.test/forum/linux/42"
        );
        assert_eq!(
            sTelegramPostText(" \t\r\n", Some("rust"), "/forum/linux/42"),
            "Без заглавия #rust\n\n/forum/linux/42"
        );
    }

    #[test]
    fn telegram_text_omits_phantom_hash_for_topic_without_tags() {
        for optTags in [None, Some("")] {
            assert_eq!(
                sTelegramPostText("A &amp; B", optTags, "/news/linux/7"),
                "A & B \n\n/news/linux/7"
            );
        }
    }

    async fn stExternalServer(
        sStatus: &str,
        sBody: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let stAddress = stListener.local_addr().expect("listener address");
        let sStatus = sStatus.to_owned();
        let sBody = sBody.to_owned();
        let hServer = tokio::spawn(async move {
            let (mut stStream, _) = stListener.accept().await.expect("test request");
            let mut vecRequest = vec![0_u8; 4096];
            let iRead = stStream.read(&mut vecRequest).await.expect("read request");
            let sRequest = String::from_utf8_lossy(&vecRequest[..iRead]).to_string();
            let sResponse = format!(
                "HTTP/1.1 {sStatus}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sBody}",
                sBody.len()
            );
            stStream
                .write_all(sResponse.as_bytes())
                .await
                .expect("write response");
            sRequest
        });
        (format!("http://{stAddress}/feed.txt"), hServer)
    }

    #[tokio::test]
    async fn external_list_adapter_returns_only_successful_bodies() {
        let (sOkUrl, hOkServer) = stExternalServer("200 OK", "one\ntwo\n").await;
        assert_eq!(
            sFetchExternalList(&reqwest::Client::new(), &sOkUrl)
                .await
                .expect("successful feed"),
            "one\ntwo\n"
        );
        assert!(
            hOkServer
                .await
                .unwrap()
                .starts_with("GET /feed.txt HTTP/1.1")
        );

        let (sErrorUrl, hErrorServer) =
            stExternalServer("503 Service Unavailable", "maintenance").await;
        let sError = sFetchExternalList(&reqwest::Client::new(), &sErrorUrl)
            .await
            .expect_err("non-success feed must not update the database")
            .to_string();
        assert!(sError.contains("503"));
        hErrorServer.await.unwrap();
    }

    #[tokio::test]
    async fn telegram_errors_never_expose_token_bearing_urls() {
        let stRequestError = reqwest::Client::new()
            .get("http://127.0.0.1:1/botSUPER_SECRET/sendMessage")
            .send()
            .await
            .expect_err("closed test port must fail");
        let sSanitized = sTelegramHttpError("Telegram request failed", &stRequestError);
        assert!(!sSanitized.contains("SUPER_SECRET"));
        assert!(!sSanitized.contains("127.0.0.1"));
        assert_eq!(sSanitized, "Telegram request failed (connect)");

        let (sUrl, hServer) = stExternalServer("200 OK", "not-json").await;
        let sSecretUrl = sUrl.replace("feed.txt", "botSUPER_SECRET/sendMessage");
        let stDecodeError = reqwest::Client::new()
            .get(sSecretUrl)
            .send()
            .await
            .expect("HTTP response")
            .json::<serde_json::Value>()
            .await
            .expect_err("invalid JSON must fail");
        let sSanitized = sTelegramHttpError("Telegram JSON decode failed", &stDecodeError);
        assert!(!sSanitized.contains("SUPER_SECRET"));
        assert!(!sSanitized.contains("127.0.0.1"));
        assert_eq!(sSanitized, "Telegram JSON decode failed (decode)");
        hServer.await.unwrap();
    }

    #[tokio::test]
    async fn telegram_adapter_uses_direct_then_configured_proxy() {
        let (sDirectUrl, hDirectServer) =
            stExternalServer("503 Service Unavailable", "direct unavailable").await;
        let sTelegramUrl = sDirectUrl.replace("feed.txt", "botTEST_TOKEN/sendMessage");
        let (sProxyUrl, hProxyServer) =
            stExternalServer("200 OK", r#"{"ok":true,"result":{"message_id":42}}"#).await;
        let cDirect = reqwest::Client::new();
        let cProxy = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(&sProxyUrl).expect("test proxy URL"))
            .build()
            .expect("proxy client");

        let stResponse = stTelegramRequest(
            &cDirect,
            Some(&cProxy),
            &sTelegramUrl,
            &[("chat_id", "@best_of_lor")],
        )
        .await
        .expect("fallback response");
        assert_eq!(stResponse.status(), reqwest::StatusCode::OK);
        assert!(
            stResponse
                .text()
                .await
                .expect("fallback body")
                .contains("message_id")
        );

        assert!(
            hDirectServer
                .await
                .unwrap()
                .starts_with("GET /botTEST_TOKEN/sendMessage?chat_id=%40best_of_lor HTTP/1.1")
        );
        let sProxyRequest = hProxyServer.await.unwrap();
        assert!(sProxyRequest.starts_with("GET http://"));
        assert!(sProxyRequest.contains("/botTEST_TOKEN/sendMessage?chat_id=%40best_of_lor"));
    }
}
