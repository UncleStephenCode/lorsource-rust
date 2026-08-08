# Service / DAO porting map

> Historical inventory. Any `db/migrations/*` path below is offline evidence,
> not an active migration; use `compat/java-db/` and
> `docs/DATABASE_COMPATIBILITY.md` for current database operations.

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
| `linux` | `OpenSearchIndexCreationService` | not ported yet | `pending` |
| `linux` | `OpenSearchIndexService` | not ported yet | `pending` |
| `linux` | `Perf4jHandlerInterceptor` | not ported yet | `pending` |
| `linux` | `PollDao` | `src/routes/api.rs`, `db/migrations/0004_current_java_schema_compat.sql` | `ported-partial` |
| `linux` | `PollPrepareService` | `src/routes/api.rs` poll boxlet/list surface | `ported-partial` |
| `linux` | `PreparedRemarkService` | not ported yet | `pending` |
| `linux` | `ProfileDao` | not ported yet | `pending` |
| `linux` | `ReactionDao` | not ported yet | `pending` |
| `linux` | `ReactionService` | `src/routes/api.rs`, `reactions_log` table | `ported-partial` |
| `linux` | `RemarkDao` | not ported yet | `pending` |
| `linux` | `SameIpDao` | not ported yet | `pending` |
| `linux` | `SameIpService` | `src/routes/admin.rs` same-IP query | `ported-partial` |
| `linux` | `ScoreUpdater` | not ported yet | `pending` |
| `linux` | `SearchService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `SearchServiceRequest` | not ported yet | `pending` |
| `linux` | `SearchServiceResponse` | not ported yet | `pending` |
| `linux` | `SecretTokenService` | `src/security.rs`, `src/routes/legacy.rs` activation handlers | `ported-partial` |
| `linux` | `SectionDao` | not ported yet | `pending` |
| `linux` | `SectionService` | not ported yet | `pending` |
| `linux` | `SlowModeChecker` | not ported yet | `pending` |
| `linux` | `StatUpdater` | not ported yet | `pending` |
| `linux` | `TagCloudDao` | not ported yet | `pending` |
| `linux` | `TagCountersUpdater` | not ported yet | `pending` |
| `linux` | `TagDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TagModificationService` | not ported yet | `pending` |
| `linux` | `TagService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `TelegramPostsDao` | not ported yet | `pending` |
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
| `linux` | `UserEventDao` | not ported yet | `pending` |
| `linux` | `UserEventPrepareService` | not ported yet | `pending` |
| `linux` | `UserEventService` | not ported yet | `pending` |
| `linux` | `UserInvitesDao` | not ported yet | `pending` |
| `linux` | `UserLogDao` | `src/audit.rs`, `user_log` table | `ported-partial` |
| `linux` | `UserLogPrepareService` | not ported yet | `pending` |
| `linux` | `UserPermissionService` | `src/security.rs`, route-level checks | `ported-partial` |
| `linux` | `UserService` | `src/routes/auth.rs`, `src/routes/users.rs`, `src/routes/legacy.rs` | `ported-partial` |
| `linux` | `UserStatisticsService` | not ported yet | `pending` |
| `linux` | `UserTagDao` | not ported yet | `pending` |
| `linux` | `UserTagService` | not ported yet | `pending` |
| `linux` | `UserpicPermissionInterceptor` | not ported yet | `pending` |
| `linux` | `WarningDao` | not ported yet | `pending` |
| `linux` | `WarningService` | `src/routes/admin.rs`, `message_warnings` table | `ported-partial` |
