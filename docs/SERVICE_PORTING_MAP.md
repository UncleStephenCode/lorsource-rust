# Service / DAO porting map

> Historical row-by-row inventory. The statuses below were generated before
> the current parity work and are not a current backlog: several rows marked
> `pending` or `scaffolded` now have production code and tests. Use
> `docs/FUNCTIONAL_COVERAGE.md`, `docs/FUNCTIONAL_COMPARISON_JAVA_RUST.md` and
> `docs/PRODUCTION_CUTOVER.md` for the current release gaps. Any
> `db/migrations/*` path below is offline evidence, not an active migration;
> use `compat/java-db/` and `docs/DATABASE_COMPATIBILITY.md` for current
> database operations.

## Re-audited current subsystems (2026-08-09)

| Java subsystem | Current Rust implementation | Evidence status |
|---|---|---|
| `AdvCounterInterceptor` / `AdvCounterActor` / `AdvCounterDao` | `src/routes/adv.rs`, `src/application/adv_counter.rs`, background transactional batch flush | ported; status/path unit coverage and stateful HTTP→`adv_counts` verification |
| `LastLoginInterceptor` / `UserDao.updateLastlogin` | global session hydration middleware in `src/auth.rs`, one-hour throttled canonical update and request-local identity cache | ported; authenticated route without `CurrentUser` statefully verified |
| Spring Security static exclusions / Tuckey cache filters | `src/security.rs`, `src/auth.rs`, `src/csrf.rs`, `src/security_headers.rs`, `src/routes/static_cache.rs` | ported; fresh-session dual-runtime cookie/cache matrix covers excluded and secured resources plus error dispatch behavior |
| `CommonContextFilter` / `DateFormats` / `head.jsp` browser bootstrap | `src/request_timezone.rs`, `src/theme_middleware.rs`, `templates/base.html` | ported; bad-zone filtering, system fallback, all four Java date modes across topic/profile/history/deleted/reaction/notification/search surfaces, and original `fixTimezone`/bundle order |
| `EditHistoryController` / `EditHistoryService` / `EditHistoryDao` | `src/application/edit_history.rs`, `src/domain/edit_history.rs`, `src/infra/postgres/edit_history_repository.rs`, `templates/history.html` | ported; type-scoped reconstruction, original diff JS/DOM, access split and `fromHistory` verified against both runtimes |
| `AddTopicChecker`, `SlowModeChecker`, `IpBlockChecker` | `src/application/topic/posting.rs`, `src/domain/topic/posting.rs`, PostgreSQL repository | ported; unit and stateful write-flow coverage |
| `CaptchaService` | `src/application/auth/mod.rs`, login/registration/comment/topic handlers | ported; negative HTTP and unit coverage |
| `CommentCreateService`, delete/edit services | `src/routes/comments.rs`, flood cache and transactional SQL | ported; unit and stateful write-flow coverage |
| `EmailService` / `ExceptionMailingActor` | `src/application/email`, `src/application/exception_reporting.rs`, `src/infra/smtp`; activation/reset and rate-limited crash reports | ported; actor-mailbox and complete reporter→SMTP sink tests pass; production MTA rehearsal remains |
| `GeoLocationService` | `src/application/geo_location.rs` plus moderator-only route with the original `ip` contract, success/error projection and non-2xx handling | ported; isolated HTTP adapter tests pass; live production egress verification remains |
| `BlackListUpdater` | `src/bootstrap/background.rs` TOR/disposable-domain jobs with advisory locks, Java-exact line handling and TOR per-row commit semantics | ported; isolated 2xx/non-2xx adapter tests pass; live production egress verification remains |
| `ReactionDao` / `ReactionService` | `src/routes/api.rs`, topic/comment widget rendering and original visibility controls | ported; unit and stateful write-flow coverage |
| `UserService` / `DeleteService` user moderation | `src/application/user`, `src/infra/postgres/user_moderation_repository.rs`, `/usermod.jsp` | ported; guarded user/profile/audit and destructive graph/event transaction coverage |
| `WhoisController`, `EditProfileController`, `EditRemarkController`, `UserFilterController` | `src/routes/users.rs`, `src/routes/legacy.rs`, profile/filter templates | ported; Java form fields, profile privacy, remarks direction/pagination, no-store private HTML and browser/JSON filter contracts covered statefully |
| `WarningService` / `WarningDao` | `src/application/warning`, `src/domain/warning`, `src/infra/postgres/warning_repository.rs`; thin `/post-warning` and `/clear-warning` adapters | ported; authorization/validation unit coverage plus ordinary score-50 author, active moderator/corrector recipients, clearing, counters and rate-limit stateful coverage |
| OpenSearch services | `src/search_index.rs`, durable filesystem spool and `src/infra/opensearch` | ported; initialized demo OpenSearch and restart-safe queue coverage |
| `SecretTokenService` | `src/security.rs` activation/reset/register-permit cryptographic formats | ported; Java fixtures and flow tests |
| `TelegramPoster` / `TelegramPostsDao` | `src/bootstrap/background.rs` direct-then-proxy publish/delete scheduler; request/decode failures redact token-bearing URLs like Java's `TelegramHttpFailedException` | ported; direct→proxy and redaction tests pass; live token/channel verification remains |
| scheduled statistics/score/cleanup jobs | `src/bootstrap/background.rs` with per-job PostgreSQL advisory locks | ported; active-scheduler production rehearsal remains |

The legacy table below is retained only as provenance for the original audit;
do not count its `pending` labels as open work without rechecking both source
trees.

Original service-like classes found: **91**

Status legend: `ported` means used by Rust handlers now; `ported-partial` means high-level behavior exists but exact side effects still differ; `scaffolded` means model/table/route exists; `pending` means Scala business logic still needs manual porting.

| Area | Class | Rust target | Status |
|---|---|---|---|
| `linux` | `AddTopicChecker` | not ported yet | `pending` |
| `linux` | `AdvCounterDao` | not ported yet | `pending` |
| `linux` | `AdvCounterInterceptor` | not ported yet | `pending` |
| `linux` | `ArchiveDao` | not ported yet | `pending` |
| `linux` | `BlackListUpdater` | not ported yet | `pending` |
| `linux` | `BoxletTopicDao` | not ported yet | `pending` |
| `linux` | `CaptchaService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `CommentCreateService` | not ported yet | `pending` |
| `linux` | `CommentDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `CommentPrepareService` | not ported yet | `pending` |
| `linux` | `CommentReadService` | not ported yet | `pending` |
| `linux` | `DeleteInfoDao` | not ported yet | `pending` |
| `linux` | `DeleteService` | not ported yet | `pending` |
| `linux` | `EditHistoryDao` | not ported yet | `pending` |
| `linux` | `EditHistoryService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `EditProfileChecker` | not ported yet | `pending` |
| `linux` | `EditTopicChecker` | not ported yet | `pending` |
| `linux` | `EmailDomainsBlockDao` | not ported yet | `pending` |
| `linux` | `EmailService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `FrozenUserChecker` | not ported yet | `pending` |
| `linux` | `GalleryPermissionInterceptor` | not ported yet | `pending` |
| `linux` | `GeoLocationService` | `src/routes/admin.rs` GeoIP surface | `scaffolded` |
| `linux` | `GroupDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `GroupInfoPrepareService` | not ported yet | `pending` |
| `linux` | `GroupListDao` | not ported yet | `pending` |
| `linux` | `GroupPermissionService` | not ported yet | `pending` |
| `linux` | `GroupService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `HSTSInterceptor` | not ported yet | `pending` |
| `linux` | `IgnoreListDao` | `src/routes/legacy.rs`, `ignore_list` table | `ported-partial` |
| `linux` | `ImageDao` | not ported yet | `pending` |
| `linux` | `ImageService` | `src/routes/legacy.rs` userpic upload + `images` table scaffold | `ported-partial` |
| `linux` | `IpBlockChecker` | not ported yet | `pending` |
| `linux` | `IpBlockDao` | `src/routes/admin.rs`, `b_ips` table | `ported-partial` |
| `linux` | `LastLoginInterceptor` | not ported yet | `pending` |
| `linux` | `LorCodeService` | not ported yet | `pending` |
| `linux` | `MemoriesDao` | `src/routes/legacy.rs`, `memories` table | `ported-partial` |
| `linux` | `MessageTextService` | not ported yet | `pending` |
| `linux` | `MoreLikeThisService` | not ported yet | `pending` |
| `linux` | `MsgbaseDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `OpenSearchIndexCreationService` | `src/search_index.rs` Java-compatible analysis/mappings | `ported` |
| `linux` | `OpenSearchIndexService` | `src/search_index.rs` topic/comment/month indexing + durable filesystem queue/retry | `ported` |
| `linux` | `Perf4jHandlerInterceptor` | not ported yet | `pending` |
| `linux` | `PollDao` | `src/routes/api.rs`, `db/migrations/0004_current_java_schema_compat.sql` | `ported-partial` |
| `linux` | `PollPrepareService` | `src/routes/api.rs` poll boxlet/list surface | `ported-partial` |
| `linux` | `PreparedRemarkService` | not ported yet | `pending` |
| `linux` | `ProfileDao` | `src/profile.rs`, `src/routes/users.rs`, Java hstore keys including `oldNotifications` | `ported-partial` |
| `linux` | `ReactionDao` | `src/routes/api.rs`, Java JSONB + `reactions_log` transaction semantics | `ported-partial` |
| `linux` | `ReactionService` | `src/routes/api.rs`, widgets, rate/visibility checks and notification side effects | `ported-partial` |
| `linux` | `RemarkDao` | not ported yet | `pending` |
| `linux` | `SameIpDao` | not ported yet | `pending` |
| `linux` | `SameIpService` | `src/routes/admin.rs` same-IP query | `ported-partial` |
| `linux` | `ScoreUpdater` | `src/bootstrap/background.rs`, original cron/SQL + advisory locks | `ported` |
| `linux` | `SearchService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `SearchServiceRequest` | not ported yet | `pending` |
| `linux` | `SearchServiceResponse` | not ported yet | `pending` |
| `linux` | `SecretTokenService` | `src/security.rs`, `src/routes/legacy.rs` activation handlers | `ported-partial` |
| `linux` | `SectionDao` | not ported yet | `pending` |
| `linux` | `SectionService` | not ported yet | `pending` |
| `linux` | `SlowModeChecker` | not ported yet | `pending` |
| `linux` | `StatUpdater` | `src/bootstrap/background.rs`, `stat_update*`/monthly/warnings | `ported` |
| `linux` | `TagCloudDao` | not ported yet | `pending` |
| `linux` | `TagCountersUpdater` | `src/bootstrap/background.rs`, counters + unused favorites | `ported` |
| `linux` | `TagDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TagModificationService` | not ported yet | `pending` |
| `linux` | `TagService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TelegramPostsDao` | `src/bootstrap/background.rs`, hot-topic post/delete workflow | `ported` |
| `linux` | `TopicDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TopicListDao` | not ported yet | `pending` |
| `linux` | `TopicListService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TopicPermissionService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `TopicPrepareService` | not ported yet | `pending` |
| `linux` | `TopicService` | not ported yet | `pending` |
| `linux` | `TopicTagDao` | not ported yet | `pending` |
| `linux` | `TopicTagService` | not ported yet | `pending` |
| `linux` | `UserAgentDao` | not ported yet | `pending` |
| `linux` | `UserDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `UserDetailsServiceImpl` | not ported yet | `pending` |
| `linux` | `UserEventDao` | `src/routes/api.rs`, `src/routes/legacy.rs`, current-reaction projection and unread transactions | `ported-partial` |
| `linux` | `UserEventPrepareService` | `src/routes/api.rs`, reaction/WATCH grouping and stale-reaction filtering | `ported-partial` |
| `linux` | `UserEventService` | `/notifications*`, `/show-replies.jsp`, realtime unread refresh | `ported-partial` |
| `linux` | `UserInvitesDao` | not ported yet | `pending` |
| `linux` | `UserLogDao` | `src/audit.rs`, `user_log` table | `ported-partial` |
| `linux` | `UserLogPrepareService` | not ported yet | `pending` |
| `linux` | `UserPermissionService` | `src/security.rs`, route-level checks | `ported-partial` |
| `linux` | `UserService` | `src/routes/auth.rs`, `src/routes/users.rs`, `src/routes/legacy.rs` | `ported-partial` |
| `linux` | `UserStatisticsService` | `src/application/user/statistics.rs`, `src/domain/user/statistics.rs`, `src/infra/opensearch/mod.rs`, `src/infra/postgres/user_statistics_repository.rs`, profile route/template | `ported`; ordinary profile uses the original two concurrent OpenSearch queries under one five-second deadline, independent incomplete recovery, and the original PostgreSQL-only ignore/comment-date values; year histogram retains the timezone-aware JSON contract |
| `linux` | `UserTagDao` | not ported yet | `pending` |
| `linux` | `UserTagService` | not ported yet | `pending` |
| `linux` | `UserpicPermissionInterceptor` | not ported yet | `pending` |
| `linux` | `WarningDao` | `src/domain/warning/repository.rs`, `src/infra/postgres/warning_repository.rs` | `ported` |
| `linux` | `WarningService` | `src/application/warning`, thin adapters in `src/routes/admin.rs` | `ported` |
