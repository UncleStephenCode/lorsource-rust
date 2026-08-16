# Score compatibility and scheduler activation

The pinned compatibility source is `lorsource-java` commit
`2ddf930005adac28077cb6ad74d1481485f44096`:

- `src/main/scala/ru/org/linux/user/ScoreUpdater.scala` defines the three
  score-related schedules;
- `src/main/scala/ru/org/linux/user/UserDao.scala` defines the updates.

The Rust production implementations are in `src/bootstrap/background.rs`.
They preserve these contracts:

- at `01:00:01` in the process system timezone on odd days of the month, add
  exactly one point per distinct author who has a non-deleted comment from the
  last two days in a non-deleted, non-`notop` topic outside groups `8404`,
  `4068`, `9326`, and `19405`; then raise `max_score` where necessary in the
  same transaction;
- at minute `15:01` of every hour, set `max_score=score` only where
  `score>max_score`;
- at minute `01:00` of every hour, block users with `score < -50`, except the
  exact nick `anonymous`, users with `max_score >= 150`, and users already
  blocked.

The automatic score query intentionally has no activation, blocked-user, or
anonymous-user filter. This is the Java behavior. Multiple qualifying
comments still grant only one point in a run.

Java `@Scheduled` declarations do not specify a zone, so Spring uses the
JVM/system timezone. Rust uses the process system timezone as well. Compose
maps the operator-facing `SCHEDULER_TIMEZONE` setting to the container's `TZ`:
local Compose defaults it to `UTC`, while the production manifest requires an
explicit IANA zone matching the original Java scheduler. Record the Java JVM
timezone before cutover; do not assume Moscow without deployment evidence.

## Production activation

The safe Compose default remains:

```text
ENABLE_BACKGROUND_JOBS=false
```

With that value, automatic score accrual, maximum-score maintenance, and
low-score blocking do not run. The startup log states this explicitly.

At cutover, set the following on **exactly one active scheduler instance**:

```text
SCHEDULER_TIMEZONE=<original-Java-IANA-zone>
ENABLE_BACKGROUND_JOBS=true
```

Keep it `false` on passive/dual-run instances and on every additional web
replica. Advisory transaction locks prevent overlapping executions, but they
do not make sequential executions by two independently scheduled replicas a
single run. The activation value is fail-closed: only the exact lowercase
literals `true` and `false` are accepted, so an operator typo cannot silently
disable score accrual.

## Focused verification

The normal test suite checks the cron and SQL source contracts. An ignored
database test exercises the production update functions in a fresh UUID
schema, closes its fixture pool, and always attempts to drop that schema
through a separate admin pool:

```bash
LOR_SCORE_DB_INTEGRATION_CONFIRM=isolated-schema \
LOR_SCORE_DB_INTEGRATION_EXPECT_DATABASE='lorsource_score_test_<unique>' \
LOR_SCORE_DB_INTEGRATION_DATABASE_URL='postgresql://USER:PASSWORD@HOST/lorsource_score_test_<unique>' \
cargo test automatic_score_jobs_match_java_in_an_isolated_schema -- --ignored
```

Create that throwaway database separately, run the proof, and drop the entire
database afterwards. Never point it at the live `lor` database, `postgres`, a
`template*` database, or a production clone: the tested production helpers use
the same database-scoped advisory-lock keys as the real scheduler. The guard
requires the connected database name to exactly match the explicit expected
name and start with `lorsource_score_test_`.

Within that disposable database, an admin pool creates/drops the UUID schema
and the fixture pool applies its UUID `search_path` after every physical
connection, including reconnects. The test verifies `current_schema()` before
calling the production update helpers and does not create or mutate objects in
`public`.
