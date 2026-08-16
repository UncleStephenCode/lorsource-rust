//! Java-compatible scheduled maintenance and external publishing jobs.
//!
//! Spring-style jobs share one FIFO in-process execution gate and retain their
//! per-job PostgreSQL advisory locks. Exactly one scheduler replica is still
//! required: advisory locks prevent overlap, not a later second execution.

use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use chrono::{DateTime, Datelike, Local, Timelike};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

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
const LOCK_UNUSED_TAGS: i64 = 0x4c4f_520e;

type TySpringSchedulerGate = Arc<Mutex<()>>;

const SCORE_CRON_HOUR: u32 = 1;
const SCORE_CRON_MINUTE: u32 = 0;
const SCORE_CRON_SECOND: u32 = 1;
const MAX_SCORE_CRON_MINUTE: u32 = 15;
const MAX_SCORE_CRON_SECOND: u32 = 1;
const LOW_SCORE_CRON_MINUTE: u32 = 1;
const LOW_SCORE_CRON_SECOND: u32 = 0;

const S_UPDATE_SCORE_SQL: &str = r#"UPDATE users SET score=score+1 WHERE id IN (
     SELECT DISTINCT comments.userid FROM comments,topics
     WHERE comments.postdate>CURRENT_TIMESTAMP-'2 days'::interval
       AND topics.id=comments.topic AND topics.groupid NOT IN (8404,4068,9326,19405)
       AND NOT comments.deleted AND NOT topics.deleted AND NOT topics.notop)"#;
const S_UPDATE_MAX_SCORE_SQL: &str = "UPDATE users SET max_score=score WHERE score>max_score";
const S_BLOCK_LOW_SCORE_SQL: &str = "UPDATE users SET blocked=true WHERE score < -50 AND nick!='anonymous' AND max_score<150 AND NOT blocked";

const HOUR: Duration = Duration::from_secs(60 * 60);
const FOUR_HOURS: Duration = Duration::from_secs(4 * 60 * 60);
const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);
const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);
const S_TOR_EXIT_LIST_URL: &str = "https://www.dan.me.uk/torlist/?exit";
const S_DISPOSABLE_DOMAINS_URL: &str =
    "https://disposable.github.io/disposable-email-domains/domains_mx.txt";

pub fn vecSpawn(stState: AppState, oShutdown: watch::Receiver<bool>) -> Vec<JoinHandle<()>> {
    let sSchedulerTimezone = std::env::var("TZ").unwrap_or_else(|_| "system-local".to_owned());
    let mut vecJobs = vec![
        stSpawnSearchQueue(stState.clone(), oShutdown.clone()),
        stSpawnAdvCounters(stState.clone(), oShutdown.clone()),
    ];
    if !stState.config.enable_background_jobs {
        tracing::info!(
            automatic_score = false,
            maximum_score = false,
            low_score_blocking = false,
            scheduler_timezone = %sSchedulerTimezone,
            "maintenance and external background jobs disabled by configuration"
        );
        return vecJobs;
    }

    tracing::info!(
        automatic_score = true,
        maximum_score = true,
        low_score_blocking = true,
        scheduler_timezone = %sSchedulerTimezone,
        "maintenance and external background jobs enabled"
    );

    // Spring's default TaskScheduler is a single-thread scheduled executor.
    // Tokio tasks retain independent clocks, but every Spring-style callback
    // waits on this one FIFO mutex and therefore never overlaps or skips.
    let oSchedulerGate = Arc::new(Mutex::new(()));
    vecJobs.extend([
        stSpawnFixed(
            "statistics",
            FIVE_MINUTES,
            TEN_MINUTES,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateStatistics(&stState.pool).await },
        ),
        stSpawnFixed(
            "group statistics",
            FIVE_MINUTES,
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateGroupStatistics(&stState.pool).await },
        ),
        stSpawnFixed(
            "tag counter recalculation",
            FIVE_MINUTES,
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vRecalculateTagCounters(&stState.pool).await },
        ),
        stSpawnFixed(
            "unused favorite tags",
            FIVE_MINUTES,
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vDeleteUnusedTags(&stState.pool).await },
        ),
        stSpawnFixed(
            "old events",
            FIVE_MINUTES,
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vCleanupOldEvents(&stState.pool).await },
        ),
        stSpawnFixed(
            "old gallery previews",
            FIVE_MINUTES,
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vCleanupGalleryPreviews(&stState).await },
        ),
        stSpawnFixed(
            "disposable email domains",
            Duration::from_secs(60),
            FOUR_HOURS,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateDisposableDomains(&stState).await },
        ),
        stSpawnFixed(
            "TOR exit nodes",
            Duration::from_secs(30 * 60),
            HOUR,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateTorExitNodes(&stState).await },
        ),
        stSpawnFixed(
            "Telegram publisher",
            Duration::from_secs(60),
            FIVE_MINUTES,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateTelegram(&stState).await },
        ),
        stSpawnHourly(
            "maximum score",
            MAX_SCORE_CRON_MINUTE,
            MAX_SCORE_CRON_SECOND,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vUpdateMaxScore(&stState.pool).await },
        ),
        stSpawnHourly(
            "low-score blocking",
            LOW_SCORE_CRON_MINUTE,
            LOW_SCORE_CRON_SECOND,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vBlockLowScoreUsers(&stState.pool).await },
        ),
        stSpawnHourly(
            "inactive accounts",
            30,
            0,
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState| async move { vDeleteInactiveAccounts(&stState.pool).await },
        ),
        stSpawnDaily(
            "score",
            SCORE_CRON_HOUR,
            SCORE_CRON_MINUTE,
            SCORE_CRON_SECOND,
            |dtCandidate| bScoreCronDay(dtCandidate.day()),
            oSchedulerGate.clone(),
            stState.clone(),
            oShutdown.clone(),
            |stState, _dtTriggeredAt| async move { vUpdateScore(&stState.pool).await },
        ),
        stSpawnDaily(
            "old userpics",
            4,
            30,
            0,
            |_dtCandidate| true,
            oSchedulerGate,
            stState.clone(),
            oShutdown,
            |stState, _dtTriggeredAt| async move { vCleanupOldUserpics(&stState).await },
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
    oSchedulerGate: TySpringSchedulerGate,
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
            vRunSpringScheduled(&oSchedulerGate, async {
                if let Err(stError) = fRun(stState.clone()).await {
                    vReportScheduledFailure(&stState, sName, &stError);
                }
            })
            .await;
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
    oSchedulerGate: TySpringSchedulerGate,
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
            vRunSpringScheduled(&oSchedulerGate, async {
                if let Err(stError) = fRun(stState.clone()).await {
                    vReportScheduledFailure(&stState, sName, &stError);
                }
            })
            .await;
        }
    })
}

fn stSpawnDaily<P, F, Fut>(
    sName: &'static str,
    iHour: u32,
    iMinute: u32,
    iSecond: u32,
    fMatchesTrigger: P,
    oSchedulerGate: TySpringSchedulerGate,
    stState: AppState,
    mut oShutdown: watch::Receiver<bool>,
    fRun: F,
) -> JoinHandle<()>
where
    P: Fn(&DateTime<Local>) -> bool + Send + Sync + 'static,
    F: Fn(AppState, DateTime<Local>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            // Resolve the precise cron instant before sleeping. Scheduled
            // executor wake-up may be delayed by VM suspension, and callback
            // execution may then wait behind an earlier Spring-style job.
            // Neither delay may change the trigger's local calendar fields.
            let dtNow = dtSchedulerNow();
            let dtTriggeredAt =
                dtNextDayMatchingAt(dtNow, iHour, iMinute, iSecond, |dtCandidate| {
                    fMatchesTrigger(dtCandidate)
                });
            let stDelay = stDurationUntil(&dtNow, &dtTriggeredAt);
            if bWaitOrShutdown(stDelay, &mut oShutdown).await {
                return;
            }
            vRunSpringScheduled(&oSchedulerGate, async {
                if let Err(stError) = fRun(stState.clone(), dtTriggeredAt).await {
                    vReportScheduledFailure(&stState, sName, &stError);
                }
            })
            .await;
        }
    })
}

async fn vRunSpringScheduled<Fut>(oSchedulerGate: &Mutex<()>, stCallback: Fut)
where
    Fut: Future<Output = ()>,
{
    // Tokio's mutex is FIFO: a callback that becomes due while another one is
    // running waits and executes afterwards, matching Spring's single thread.
    let _stSchedulerGuard = oSchedulerGate.lock().await;
    stCallback.await;
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
        // Java groups periodic reports by `ex.getClass`, not by callback name.
        // Recover the concrete adapter/database class from anyhow's source
        // chain where possible; unknown application errors share the anyhow
        // class instead of being split into one rate-limit bucket per job.
        sType: sScheduledErrorClass(stError).to_owned(),
        sBody: format!("Periodic task failed\n\nJob: {sName}\n{sError}"),
    }
}

fn sScheduledErrorClass(stError: &anyhow::Error) -> &'static str {
    for stCause in stError.chain() {
        if stCause.is::<sqlx::Error>() {
            return "sqlx::Error";
        }
        if stCause.is::<reqwest::Error>() {
            return "reqwest::Error";
        }
        if stCause.is::<std::io::Error>() {
            return "std::io::Error";
        }
        if stCause.is::<serde_json::Error>() {
            return "serde_json::Error";
        }
        if stCause.is::<image::ImageError>() {
            return "image::ImageError";
        }
    }
    "anyhow::Error"
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
    stUntilNextHourAt(dtSchedulerNow(), iMinute, iSecond)
}

#[cfg(test)]
fn stUntilNextDay(iHour: u32, iMinute: u32, iSecond: u32) -> Duration {
    stUntilNextDayAt(dtSchedulerNow(), iHour, iMinute, iSecond)
}

fn stUntilNextHourAt<Tz>(dtNow: DateTime<Tz>, iMinute: u32, iSecond: u32) -> Duration
where
    Tz: chrono::TimeZone,
{
    assert!(iMinute < 60 && iSecond < 60, "valid scheduler time");
    let dtNext = dtNextLocalMatch(dtNow.clone(), 3 * 24 * 60 * 60, |dtCandidate| {
        dtCandidate.minute() == iMinute && dtCandidate.second() == iSecond
    });
    stDurationUntil(&dtNow, &dtNext)
}

#[cfg(test)]
fn stUntilNextDayAt<Tz>(dtNow: DateTime<Tz>, iHour: u32, iMinute: u32, iSecond: u32) -> Duration
where
    Tz: chrono::TimeZone,
{
    let dtNext = dtNextDayMatchingAt(dtNow.clone(), iHour, iMinute, iSecond, |_dtCandidate| true);
    stDurationUntil(&dtNow, &dtNext)
}

fn dtNextDayMatchingAt<Tz, F>(
    dtNow: DateTime<Tz>,
    iHour: u32,
    iMinute: u32,
    iSecond: u32,
    fMatchesDate: F,
) -> DateTime<Tz>
where
    Tz: chrono::TimeZone,
    F: Fn(&DateTime<Tz>) -> bool,
{
    assert!(
        iHour < 24 && iMinute < 60 && iSecond < 60,
        "valid scheduler time"
    );
    // Seven days also covers an odd-day trigger whose local wall time is
    // nonexistent during a timezone transition: the next odd date still
    // remains inside this search window.
    dtNextLocalMatch(dtNow, 7 * 24 * 60 * 60, |dtCandidate| {
        dtCandidate.hour() == iHour
            && dtCandidate.minute() == iMinute
            && dtCandidate.second() == iSecond
            && fMatchesDate(dtCandidate)
    })
}

fn stDurationUntil<Tz>(dtNow: &DateTime<Tz>, dtNext: &DateTime<Tz>) -> Duration
where
    Tz: chrono::TimeZone,
{
    dtNext
        .clone()
        .signed_duration_since(dtNow)
        .to_std()
        .expect("next cron instant is later than now")
}

fn dtNextLocalMatch<Tz, F>(dtNow: DateTime<Tz>, iSearchSeconds: usize, fMatches: F) -> DateTime<Tz>
where
    Tz: chrono::TimeZone,
    F: Fn(&DateTime<Tz>) -> bool,
{
    // Search real instants but compare local calendar fields. This preserves
    // wall-clock cron semantics through DST: nonexistent times are skipped,
    // and both occurrences of an ambiguous time remain eligible.
    // Subtract on the instant timeline instead of calling `with_nanosecond`:
    // rebuilding an ambiguous local time would discard which overlap offset
    // the previous Spring execution used.
    let mut dtCandidate = dtNow.clone()
        - chrono::Duration::nanoseconds(i64::from(dtNow.nanosecond()))
        + chrono::Duration::seconds(1);
    for _ in 0..iSearchSeconds {
        if fMatches(&dtCandidate) {
            return dtCandidate;
        }
        dtCandidate += chrono::Duration::seconds(1);
    }
    panic!("no matching local cron instant within search window")
}

fn dtSchedulerNow() -> DateTime<Local> {
    // Spring @Scheduled without an explicit zone uses the JVM/system default.
    // Compose maps the operator-facing SCHEDULER_TIMEZONE to container TZ.
    Local::now()
}

const fn bScoreCronDay(iDayOfMonth: u32) -> bool {
    iDayOfMonth % 2 == 1
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

async fn vRecalculateTagCounters(oPool: &PgPool) -> anyhow::Result<()> {
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
    stTransaction.commit().await?;
    Ok(())
}

async fn vDeleteUnusedTags(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_UNUSED_TAGS).await? {
        return Ok(());
    }
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
    sqlx::query(S_UPDATE_SCORE_SQL)
        .execute(&mut *stTransaction)
        .await?;
    sqlx::query(S_UPDATE_MAX_SCORE_SQL)
        .execute(&mut *stTransaction)
        .await?;
    stTransaction.commit().await?;
    Ok(())
}

async fn vUpdateMaxScore(oPool: &PgPool) -> anyhow::Result<()> {
    vExecuteLocked(oPool, LOCK_MAX_SCORE, S_UPDATE_MAX_SCORE_SQL).await
}

async fn vBlockLowScoreUsers(oPool: &PgPool) -> anyhow::Result<()> {
    vExecuteLocked(oPool, LOCK_LOW_SCORE, S_BLOCK_LOW_SCORE_SQL).await
}

async fn vDeleteInactiveAccounts(oPool: &PgPool) -> anyhow::Result<()> {
    let mut stTransaction = oPool.begin().await?;
    if !bLock(&mut stTransaction, LOCK_INACTIVE_USERS).await? {
        return Ok(());
    }
    sqlx::query(
        r#"DELETE FROM user_events WHERE userid IN (
             SELECT id FROM users WHERE NOT activated AND NOT blocked
               AND regdate<CURRENT_TIMESTAMP-'12 hours'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM topic_users_notified WHERE userid IN (
             SELECT id FROM users WHERE NOT activated AND NOT blocked
               AND regdate<CURRENT_TIMESTAMP-'12 hours'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    let iRegular = sqlx::query(
        r#"DELETE FROM users WHERE NOT activated AND NOT blocked
           AND regdate<CURRENT_TIMESTAMP-'12 hours'::interval"#,
    )
    .execute(&mut *stTransaction)
    .await?
    .rows_affected();

    sqlx::query(
        r#"DELETE FROM ban_info WHERE userid IN (
             SELECT id FROM users WHERE NOT activated
               AND regdate<CURRENT_TIMESTAMP-'30 days'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events WHERE userid IN (
             SELECT id FROM users WHERE NOT activated
               AND regdate<CURRENT_TIMESTAMP-'30 days'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM topic_users_notified WHERE userid IN (
             SELECT id FROM users WHERE NOT activated
               AND regdate<CURRENT_TIMESTAMP-'30 days'::interval)"#,
    )
    .execute(&mut *stTransaction)
    .await?;
    let iBlocked = sqlx::query(
        r#"DELETE FROM users WHERE NOT activated
           AND regdate<CURRENT_TIMESTAMP-'30 days'::interval"#,
    )
    .execute(&mut *stTransaction)
    .await?
    .rows_affected();
    stTransaction.commit().await?;
    tracing::info!(
        regular = iRegular,
        blocked = iBlocked,
        "deleted inactive accounts"
    );
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
    use std::{
        sync::{Arc, Mutex as StdMutex},
        time::Duration,
    };

    use chrono::{Datelike, TimeZone, Timelike};
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Notify, oneshot},
    };

    use super::{
        LOW_SCORE_CRON_MINUTE, LOW_SCORE_CRON_SECOND, MAX_SCORE_CRON_MINUTE, MAX_SCORE_CRON_SECOND,
        S_BLOCK_LOW_SCORE_SQL, S_UPDATE_MAX_SCORE_SQL, S_UPDATE_SCORE_SQL, SCORE_CRON_HOUR,
        SCORE_CRON_MINUTE, SCORE_CRON_SECOND, bScoreCronDay, dtNextDayMatchingAt, dtSchedulerNow,
        sFetchExternalList, sTelegramHttpError, sTelegramPostText, stScheduledFailureReport,
        stTelegramRequest, stUntilNextDay, stUntilNextDayAt, stUntilNextHour, vBlockLowScoreUsers,
        vRunSpringScheduled, vUpdateMaxScore, vUpdateScore, vecJavaLines,
    };

    #[test]
    fn score_schedules_match_spring_cron_contract() {
        assert_eq!(
            (SCORE_CRON_HOUR, SCORE_CRON_MINUTE, SCORE_CRON_SECOND),
            (1, 0, 1),
            "ScoreUpdater.updateScore: 1 0 1 */2 * *"
        );
        assert_eq!(
            (MAX_SCORE_CRON_MINUTE, MAX_SCORE_CRON_SECOND),
            (15, 1),
            "ScoreUpdater.updateMaxScore: 1 15 * * * *"
        );
        assert_eq!(
            (LOW_SCORE_CRON_MINUTE, LOW_SCORE_CRON_SECOND),
            (1, 0),
            "ScoreUpdater.blockUsers: 0 1 * * * *"
        );
        assert_eq!(
            (1..=31)
                .filter(|iDayOfMonth| bScoreCronDay(*iDayOfMonth))
                .collect::<Vec<_>>(),
            vec![1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31]
        );
    }

    #[test]
    fn score_sql_matches_java_user_dao_contract() {
        assert_eq!(
            S_UPDATE_SCORE_SQL,
            r#"UPDATE users SET score=score+1 WHERE id IN (
     SELECT DISTINCT comments.userid FROM comments,topics
     WHERE comments.postdate>CURRENT_TIMESTAMP-'2 days'::interval
       AND topics.id=comments.topic AND topics.groupid NOT IN (8404,4068,9326,19405)
       AND NOT comments.deleted AND NOT topics.deleted AND NOT topics.notop)"#
        );
        assert_eq!(
            S_UPDATE_MAX_SCORE_SQL,
            "UPDATE users SET max_score=score WHERE score>max_score"
        );
        assert_eq!(
            S_BLOCK_LOW_SCORE_SQL,
            "UPDATE users SET blocked=true WHERE score < -50 AND nick!='anonymous' AND max_score<150 AND NOT blocked"
        );
        for sNonJavaFilter in ["activated", "passwd", "lastlogin"] {
            assert!(
                !S_UPDATE_SCORE_SQL.contains(sNonJavaFilter),
                "automatic score must not acquire a Rust-only {sNonJavaFilter} filter"
            );
        }
    }

    #[test]
    fn startup_logs_make_score_job_activation_explicit() {
        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        for sField in [
            "automatic_score = false",
            "maximum_score = false",
            "low_score_blocking = false",
            "automatic_score = true",
            "maximum_score = true",
            "low_score_blocking = true",
        ] {
            assert!(
                sProduction.contains(sField),
                "missing startup log: {sField}"
            );
        }
    }

    #[test]
    fn spring_scheduler_uses_system_timezone_and_fixes_the_trigger_before_sleeping() {
        let _dtSystemLocal: chrono::DateTime<chrono::Local> = dtSchedulerNow();
        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        assert!(sProduction.contains("Local::now()"));
        assert!(!sProduction.contains("use chrono_tz::Europe::Moscow"));
        assert!(!sProduction.contains("with_timezone(&Moscow)"));

        let sDaily = sProduction
            .split("fn stSpawnDaily")
            .nth(1)
            .and_then(|sSource| sSource.split("async fn vRunSpringScheduled").next())
            .expect("daily scheduler source");
        let iTriggerCapture = sDaily
            .find("dtNextDayMatchingAt(")
            .expect("exact local cron trigger calculation");
        let iSchedulerSleep = sDaily
            .find("if bWaitOrShutdown(stDelay, &mut oShutdown).await")
            .expect("scheduler sleep");
        let iExecutionGate = sDaily
            .find("vRunSpringScheduled(&oSchedulerGate")
            .expect("shared execution gate");
        assert!(
            iTriggerCapture < iSchedulerSleep && iSchedulerSleep < iExecutionGate,
            "exact trigger must be fixed before scheduler sleep and FIFO queueing"
        );
        assert!(sProduction.contains("|dtCandidate| bScoreCronDay(dtCandidate.day())"));
        assert!(
            !sProduction.contains("if bScoreCronDay"),
            "odd-day selection belongs in the cron matcher, not a delayed callback"
        );
    }

    #[test]
    fn score_scheduler_preserves_an_exact_odd_trigger_across_long_delays() {
        let stTimezone = chrono_tz::UTC;
        let dtBeforeTrigger = stTimezone
            .with_ymd_and_hms(2026, 8, 16, 23, 0, 0)
            .single()
            .expect("UTC time");
        let dtTriggeredAt = dtNextDayMatchingAt(
            dtBeforeTrigger,
            SCORE_CRON_HOUR,
            SCORE_CRON_MINUTE,
            SCORE_CRON_SECOND,
            |dtCandidate| bScoreCronDay(dtCandidate.day()),
        );
        assert_eq!(dtTriggeredAt.to_rfc3339(), "2026-08-17T01:00:01+00:00");

        // Model both an overdue timer after VM suspension and a subsequent
        // wait behind a long-running callback. The callback must still carry
        // the precomputed August 17 trigger, even though wall time is now an
        // even day on which a wake-time predicate would wrongly skip it.
        let dtWakeAfterSuspension = dtTriggeredAt + chrono::Duration::hours(26);
        let dtGateReleased = dtTriggeredAt + chrono::Duration::hours(30);
        assert_eq!(dtWakeAfterSuspension.day(), 18);
        assert_eq!(dtGateReleased.day(), 18);
        assert!(bScoreCronDay(dtTriggeredAt.day()));

        // Once that delayed callback completes, lenient Spring cron semantics
        // select the next genuine odd trigger and do not queue an even day.
        let dtNextTriggeredAt = dtNextDayMatchingAt(
            dtGateReleased,
            SCORE_CRON_HOUR,
            SCORE_CRON_MINUTE,
            SCORE_CRON_SECOND,
            |dtCandidate| bScoreCronDay(dtCandidate.day()),
        );
        assert_eq!(dtNextTriggeredAt.to_rfc3339(), "2026-08-19T01:00:01+00:00");
    }

    #[tokio::test]
    async fn spring_scheduler_gate_queues_low_score_until_score_callback_finishes() {
        let oSchedulerGate = Arc::new(tokio::sync::Mutex::new(()));
        let vecEvents = Arc::new(StdMutex::new(Vec::<&'static str>::new()));
        let oReleaseScore = Arc::new(Notify::new());
        let (txScoreStarted, rxScoreStarted) = oneshot::channel();

        let hScore = tokio::spawn({
            let oSchedulerGate = oSchedulerGate.clone();
            let vecEvents = vecEvents.clone();
            let oReleaseScore = oReleaseScore.clone();
            async move {
                vRunSpringScheduled(&oSchedulerGate, async move {
                    vecEvents.lock().unwrap().push("score-start");
                    txScoreStarted.send(()).unwrap();
                    oReleaseScore.notified().await;
                    // Failure reporting is part of this serialized callback in
                    // production; model its tail before releasing the gate.
                    vecEvents.lock().unwrap().push("score-report-finished");
                })
                .await;
            }
        });
        rxScoreStarted.await.expect("score callback started");

        let (txLowScoreAttempted, rxLowScoreAttempted) = oneshot::channel();
        let (txLowScoreStarted, mut rxLowScoreStarted) = oneshot::channel();
        let hLowScore = tokio::spawn({
            let oSchedulerGate = oSchedulerGate.clone();
            let vecEvents = vecEvents.clone();
            async move {
                txLowScoreAttempted.send(()).unwrap();
                vRunSpringScheduled(&oSchedulerGate, async move {
                    vecEvents.lock().unwrap().push("low-score-start");
                    txLowScoreStarted.send(()).unwrap();
                    vecEvents.lock().unwrap().push("low-score-finished");
                })
                .await;
            }
        });
        rxLowScoreAttempted
            .await
            .expect("low-score callback attempted the shared gate");
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut rxLowScoreStarted)
                .await
                .is_err(),
            "low-score callback overlapped the still-running score callback"
        );

        oReleaseScore.notify_one();
        tokio::time::timeout(Duration::from_secs(1), hScore)
            .await
            .expect("score callback completion timeout")
            .expect("score callback task");
        tokio::time::timeout(Duration::from_secs(1), &mut rxLowScoreStarted)
            .await
            .expect("queued low-score callback timeout")
            .expect("queued low-score callback start");
        tokio::time::timeout(Duration::from_secs(1), hLowScore)
            .await
            .expect("low-score callback completion timeout")
            .expect("low-score callback task");
        assert_eq!(
            *vecEvents.lock().unwrap(),
            vec![
                "score-start",
                "score-report-finished",
                "low-score-start",
                "low-score-finished",
            ]
        );
    }

    async fn vTestExecute(oPool: &PgPool, sSql: &str) -> anyhow::Result<()> {
        sqlx::query(sqlx::AssertSqlSafe(sSql.to_owned()))
            .execute(oPool)
            .await?;
        Ok(())
    }

    fn vValidateDisposableScoreDatabase(
        sCurrentDatabase: &str,
        sExpectedDatabase: &str,
    ) -> anyhow::Result<()> {
        let sLower = sCurrentDatabase.to_ascii_lowercase();
        anyhow::ensure!(
            sCurrentDatabase == sExpectedDatabase,
            "connected database {sCurrentDatabase:?} does not match LOR_SCORE_DB_INTEGRATION_EXPECT_DATABASE {sExpectedDatabase:?}"
        );
        anyhow::ensure!(
            !matches!(sLower.as_str(), "lor" | "postgres") && !sLower.starts_with("template"),
            "refusing to run the score contract against protected database {sCurrentDatabase:?}"
        );
        anyhow::ensure!(
            sLower.starts_with("lorsource_score_test_"),
            "disposable score database name must start with lorsource_score_test_; got {sCurrentDatabase:?}"
        );
        Ok(())
    }

    #[test]
    fn score_database_guard_rejects_live_and_mismatched_names() {
        vValidateDisposableScoreDatabase(
            "lorsource_score_test_20260816",
            "lorsource_score_test_20260816",
        )
        .expect("matching disposable database");
        for (sCurrent, sExpected) in [
            ("lor", "lor"),
            ("postgres", "postgres"),
            ("template1", "template1"),
            ("production_clone", "production_clone"),
            ("lorsource_score_test_one", "lorsource_score_test_two"),
        ] {
            assert!(
                vValidateDisposableScoreDatabase(sCurrent, sExpected).is_err(),
                "unsafe database pair was accepted: {sCurrent:?}/{sExpected:?}"
            );
        }
    }

    async fn vCreateScoreFixture(oPool: &PgPool) -> anyhow::Result<()> {
        for sSql in [
            r#"CREATE TABLE users(
                   id integer PRIMARY KEY,
                   nick text NOT NULL,
                   score integer NOT NULL,
                   max_score integer NOT NULL,
                   blocked boolean NOT NULL,
                   block_updates integer NOT NULL DEFAULT 0
               )"#,
            r#"CREATE FUNCTION count_block_update() RETURNS trigger
               LANGUAGE plpgsql AS $$
               BEGIN
                   NEW.block_updates=OLD.block_updates+1;
                   RETURN NEW;
               END
               $$"#,
            r#"CREATE TRIGGER count_block_update
               BEFORE UPDATE OF blocked ON users
               FOR EACH ROW EXECUTE FUNCTION count_block_update()"#,
            r#"CREATE TABLE topics(
                   id integer PRIMARY KEY,
                   groupid integer NOT NULL,
                   deleted boolean NOT NULL,
                   notop boolean NOT NULL
               )"#,
            r#"CREATE TABLE comments(
                   id integer PRIMARY KEY,
                   userid integer NOT NULL,
                   topic integer NOT NULL,
                   postdate timestamptz NOT NULL,
                   deleted boolean NOT NULL
               )"#,
            r#"INSERT INTO users(id,nick,score,max_score,blocked) VALUES
                   (2,'anonymous',0,0,false),
                   (10,'two-comments',10,5,false),
                   (11,'old-comment',5,5,false),
                   (12,'deleted-comment',5,5,false),
                   (13,'nontech-8404',5,5,false),
                   (14,'nontech-4068',5,5,false),
                   (15,'nontech-9326',5,5,false),
                   (16,'nontech-19405',5,5,false),
                   (17,'deleted-topic',5,5,false),
                   (18,'notop-topic',5,5,false),
                   (19,'already-blocked-but-active',6,6,true),
                   (30,'max-only',40,10,false),
                   (40,'below-threshold',-51,149,false),
                   (41,'at-threshold',-50,149,false),
                   (42,'anonymous',-999,0,false),
                   (43,'max-guard',-51,150,false),
                   (44,'already-blocked',-51,149,true),
                   (45,'Anonymous',-51,149,false)"#,
            r#"INSERT INTO topics(id,groupid,deleted,notop) VALUES
                   (100,1,false,false),
                   (101,1,false,false),
                   (102,1,false,false),
                   (103,8404,false,false),
                   (104,4068,false,false),
                   (105,9326,false,false),
                   (106,19405,false,false),
                   (107,1,true,false),
                   (108,1,false,true),
                   (109,1,false,false),
                   (110,1,false,false)"#,
            r#"INSERT INTO comments(id,userid,topic,postdate,deleted) VALUES
                   (1000,2,100,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1001,10,100,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1002,10,101,CURRENT_TIMESTAMP-'2 hour'::interval,false),
                   (1003,11,102,CURRENT_TIMESTAMP-'3 days'::interval,false),
                   (1004,12,102,CURRENT_TIMESTAMP-'1 hour'::interval,true),
                   (1005,13,103,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1006,14,104,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1007,15,105,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1008,16,106,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1009,17,107,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1010,18,108,CURRENT_TIMESTAMP-'1 hour'::interval,false),
                   (1011,19,109,CURRENT_TIMESTAMP-'1 hour'::interval,false)"#,
        ] {
            vTestExecute(oPool, sSql).await?;
        }
        Ok(())
    }

    async fn stUserScore(oPool: &PgPool, iUserId: i32) -> anyhow::Result<(i32, i32)> {
        Ok(
            sqlx::query_as("SELECT score,max_score FROM users WHERE id=$1")
                .bind(iUserId)
                .fetch_one(oPool)
                .await?,
        )
    }

    #[tokio::test]
    #[ignore = "requires a separately created lorsource_score_test_* throwaway database"]
    async fn automatic_score_jobs_match_java_in_an_isolated_schema() {
        assert_eq!(
            std::env::var("LOR_SCORE_DB_INTEGRATION_CONFIRM").as_deref(),
            Ok("isolated-schema"),
            "set LOR_SCORE_DB_INTEGRATION_CONFIRM=isolated-schema"
        );
        let sDatabaseUrl = std::env::var("LOR_SCORE_DB_INTEGRATION_DATABASE_URL").expect(
            "set LOR_SCORE_DB_INTEGRATION_DATABASE_URL to the selected PostgreSQL database",
        );
        let sExpectedDatabase = std::env::var("LOR_SCORE_DB_INTEGRATION_EXPECT_DATABASE")
            .expect("set LOR_SCORE_DB_INTEGRATION_EXPECT_DATABASE to the disposable database name");
        let oAdminPool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable PostgreSQL database must be reachable");
        let sCurrentDatabase: String = sqlx::query_scalar("SELECT current_database()::text")
            .fetch_one(&oAdminPool)
            .await
            .expect("read current disposable database name");
        vValidateDisposableScoreDatabase(&sCurrentDatabase, &sExpectedDatabase)
            .expect("score test requires a named throwaway database, never the live/main database");

        let sSchema = format!("score_contract_{}", uuid::Uuid::new_v4().simple());
        vTestExecute(&oAdminPool, &format!("CREATE SCHEMA {sSchema}"))
            .await
            .expect("temporary UUID schema must be creatable");

        let stResult = async {
            let sConnectionSchema = sSchema.clone();
            let oPool = PgPoolOptions::new()
                .max_connections(2)
                .after_connect(move |oConnection, _stMetadata| {
                    let sConnectionSchema = sConnectionSchema.clone();
                    Box::pin(async move {
                        sqlx::query("SELECT set_config('search_path',$1,false)")
                            .bind(sConnectionSchema)
                            .execute(&mut *oConnection)
                            .await?;
                        Ok(())
                    })
                })
                .connect(&sDatabaseUrl)
                .await?;
            let stTestResult = async {
                let sCurrentSchema: String = sqlx::query_scalar("SELECT current_schema()::text")
                    .fetch_one(&oPool)
                    .await?;
                anyhow::ensure!(
                    sCurrentSchema == sSchema,
                    "fixture pool escaped UUID schema: {sCurrentSchema:?}"
                );
                vCreateScoreFixture(&oPool).await?;

                vUpdateScore(&oPool).await?;
                anyhow::ensure!(stUserScore(&oPool, 2).await? == (1, 1));
                anyhow::ensure!(
                    stUserScore(&oPool, 10).await? == (11, 11),
                    "two qualifying comments must still yield one point per run"
                );
                for iExcludedUser in 11..=18 {
                    anyhow::ensure!(
                        stUserScore(&oPool, iExcludedUser).await? == (5, 5),
                        "excluded user {iExcludedUser} acquired automatic score"
                    );
                }
                anyhow::ensure!(
                    stUserScore(&oPool, 19).await? == (7, 7),
                    "Java does not exclude already-blocked authors from automatic score"
                );
                anyhow::ensure!(
                    stUserScore(&oPool, 30).await? == (40, 40),
                    "the score run must synchronize max_score in the same operation"
                );

                vTestExecute(&oPool, "UPDATE users SET score=50,max_score=40 WHERE id=30").await?;
                vUpdateMaxScore(&oPool).await?;
                anyhow::ensure!(stUserScore(&oPool, 30).await? == (50, 50));
                vTestExecute(&oPool, "UPDATE users SET score=20 WHERE id=30").await?;
                vUpdateMaxScore(&oPool).await?;
                anyhow::ensure!(
                    stUserScore(&oPool, 30).await? == (20, 50),
                    "max_score must be monotonic when score falls"
                );

                vBlockLowScoreUsers(&oPool).await?;
                let vecBlockStates = sqlx::query_as::<_, (i32, bool, i32)>(
                    "SELECT id,blocked,block_updates FROM users WHERE id BETWEEN 40 AND 45 ORDER BY id",
                )
                .fetch_all(&oPool)
                .await?;
                anyhow::ensure!(
                    vecBlockStates
                        == vec![
                            (40, true, 1),
                            (41, false, 0),
                            (42, false, 0),
                            (43, false, 0),
                            (44, true, 0),
                            (45, true, 1),
                        ],
                    "low-score threshold/nick/max/already-blocked guards differ: {vecBlockStates:?}"
                );
                Ok::<_, anyhow::Error>(())
            }
            .await;
            oPool.close().await;
            stTestResult
        }
        .await;

        // Cleanup is deliberately performed through the separate admin pool,
        // even when setup/assertions inside stResult fail.
        let stDropResult =
            vTestExecute(&oAdminPool, &format!("DROP SCHEMA {sSchema} CASCADE")).await;
        oAdminPool.close().await;
        stDropResult.expect("temporary UUID schema cleanup must succeed");
        stResult.expect("automatic score database contract");
    }

    #[test]
    fn external_lists_preserve_java_lines_iterator_values() {
        assert_eq!(
            vecJavaLines(" one\n\n# note\r\n two \n"),
            vec![" one", "", "# note", " two "]
        );
    }

    #[test]
    fn scheduler_delays_find_a_future_local_match() {
        let stHourly = stUntilNextHour(15, 1);
        let stDaily = stUntilNextDay(4, 30, 0);
        assert!(stHourly > Duration::ZERO && stHourly <= Duration::from_secs(3 * 24 * 3600));
        assert!(stDaily > Duration::ZERO && stDaily <= Duration::from_secs(3 * 24 * 3600));
    }

    #[test]
    fn daily_scheduler_preserves_wall_time_across_dst_transitions() {
        let stTimezone = chrono_tz::Europe::Berlin;

        // 2026-03-29 02:30 does not exist. A local-calendar cron skips that
        // occurrence and next fires at 02:30 on March 30, not at 03:30 on
        // March 29 after adding a fixed 24-hour duration.
        let dtBeforeSpringForward = stTimezone
            .with_ymd_and_hms(2026, 3, 28, 3, 0, 0)
            .single()
            .expect("unambiguous Berlin time");
        let stSpringDelay = stUntilNextDayAt(dtBeforeSpringForward, 2, 30, 0);
        assert_eq!(stSpringDelay, Duration::from_secs(46 * 3600 + 30 * 60));
        let dtAfterSpringForward = dtBeforeSpringForward
            + chrono::Duration::from_std(stSpringDelay).expect("chrono duration");
        assert_eq!(
            (
                dtAfterSpringForward.year(),
                dtAfterSpringForward.month(),
                dtAfterSpringForward.day(),
                dtAfterSpringForward.hour(),
                dtAfterSpringForward.minute(),
            ),
            (2026, 3, 30, 2, 30)
        );

        // The autumn day is 25 hours long. Reconstructing the local match
        // preserves 04:30 instead of drifting to 03:30 or 05:30.
        let dtBeforeFallBack = stTimezone
            .with_ymd_and_hms(2026, 10, 24, 5, 0, 0)
            .single()
            .expect("unambiguous Berlin time");
        let stFallDelay = stUntilNextDayAt(dtBeforeFallBack, 4, 30, 0);
        assert_eq!(stFallDelay, Duration::from_secs(24 * 3600 + 30 * 60));
        let dtAfterFallBack =
            dtBeforeFallBack + chrono::Duration::from_std(stFallDelay).expect("chrono duration");
        assert_eq!(
            (
                dtAfterFallBack.year(),
                dtAfterFallBack.month(),
                dtAfterFallBack.day(),
                dtAfterFallBack.hour(),
                dtAfterFallBack.minute(),
            ),
            (2026, 10, 25, 4, 30)
        );

        // Spring 6.2.19 CronExpression emits both real instants in an
        // overlap. These seeds reproduce `0 30 2 * * *` in Europe/Berlin:
        // first 02:30 CEST, then (after that callback) 02:30 CET.
        let dtBeforeFirstOverlap = stTimezone
            .with_ymd_and_hms(2026, 10, 25, 1, 59, 59)
            .single()
            .expect("unambiguous Berlin time");
        let stFirstOverlapDelay = stUntilNextDayAt(dtBeforeFirstOverlap, 2, 30, 0);
        assert_eq!(stFirstOverlapDelay, Duration::from_secs(30 * 60 + 1));
        let dtFirstOverlap = dtBeforeFirstOverlap
            + chrono::Duration::from_std(stFirstOverlapDelay).expect("chrono duration");
        assert_eq!(dtFirstOverlap.to_rfc3339(), "2026-10-25T02:30:00+02:00");

        let dtAfterFirstOverlap = dtFirstOverlap + chrono::Duration::seconds(1);
        let stSecondOverlapDelay = stUntilNextDayAt(dtAfterFirstOverlap, 2, 30, 0);
        assert_eq!(stSecondOverlapDelay, Duration::from_secs(60 * 60 - 1));
        let dtSecondOverlap = dtAfterFirstOverlap
            + chrono::Duration::from_std(stSecondOverlapDelay).expect("chrono duration");
        assert_eq!(dtSecondOverlap.to_rfc3339(), "2026-10-25T02:30:00+01:00");
    }

    #[test]
    fn every_spring_style_scheduler_loop_reports_and_redacts_failures() {
        let stError =
            anyhow::anyhow!("request https://api.telegram.org/botSUPER_SECRET/sendMessage failed")
                .context("publisher adapter");
        let stReport =
            stScheduledFailureReport("Telegram publisher", &stError, Some("SUPER_SECRET"));
        assert_eq!(stReport.sType, "anyhow::Error");
        assert!(stReport.sBody.contains("publisher adapter"));
        assert!(stReport.sBody.contains("Periodic task failed"));
        assert!(!stReport.sBody.contains("SUPER_SECRET"));
        assert!(stReport.sBody.contains("[REDACTED]"));

        let stIoError = anyhow::Error::new(std::io::Error::other("disk unavailable"))
            .context("preview cleanup");
        assert_eq!(
            stScheduledFailureReport("old gallery previews", &stIoError, None).sType,
            "std::io::Error"
        );

        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        assert_eq!(
            sProduction
                .matches("vReportScheduledFailure(&stState, sName, &stError)")
                .count(),
            3,
            "fixed-delay, hourly and daily Spring-style loops must all report"
        );
        assert_eq!(
            sProduction
                .matches("vRunSpringScheduled(&oSchedulerGate")
                .count(),
            3,
            "fixed-delay, hourly and daily loops must use the shared FIFO gate"
        );
        assert_eq!(
            sProduction
                .matches("oSchedulerGate: TySpringSchedulerGate")
                .count(),
            3,
            "every Spring-style spawner must receive the one shared gate"
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
    fn tag_callbacks_keep_java_independent_transaction_boundaries() {
        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        assert!(sProduction.contains("\"tag counter recalculation\""));
        assert!(sProduction.contains("\"unused favorite tags\""));
        assert!(sProduction.contains("vRecalculateTagCounters(&stState.pool)"));
        assert!(sProduction.contains("vDeleteUnusedTags(&stState.pool)"));

        let sRecalculate = sProduction
            .split("async fn vRecalculateTagCounters")
            .nth(1)
            .and_then(|sSource| sSource.split("async fn vDeleteUnusedTags").next())
            .expect("recalculation callback");
        let sDelete = sProduction
            .split("async fn vDeleteUnusedTags")
            .nth(1)
            .and_then(|sSource| sSource.split("async fn vCleanupOldEvents").next())
            .expect("unused-tag callback");
        for sCallback in [sRecalculate, sDelete] {
            assert!(sCallback.contains("oPool.begin().await?"));
            assert!(sCallback.contains("stTransaction.commit().await?"));
        }
        assert!(!sRecalculate.contains("DELETE FROM user_tags"));
        assert!(!sDelete.contains("UPDATE tags_values"));
    }

    #[test]
    fn inactive_cleanup_keeps_java_all_or_nothing_batch_sql() {
        let sProduction = include_str!("background.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        let sCleanup = sProduction
            .split("async fn vDeleteInactiveAccounts")
            .nth(1)
            .and_then(|sSource| sSource.split("async fn vExecuteLocked").next())
            .expect("inactive cleanup callback");
        assert!(sCleanup.contains("NOT activated AND NOT blocked"));
        assert!(sCleanup.contains("'12 hours'::interval"));
        assert!(sCleanup.contains("'30 days'::interval"));
        assert_eq!(sCleanup.matches("DELETE FROM users WHERE").count(), 2);
        assert!(!sCleanup.contains("NOT EXISTS"));
        assert!(!sCleanup.contains("user_settings"));
        assert_eq!(sCleanup.matches("stTransaction.commit().await?").count(), 1);
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
