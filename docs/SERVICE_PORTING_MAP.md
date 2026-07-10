# Service / DAO porting map

Original service-like classes found: **91**

Status legend: `ported` means used by Rust handlers now; `scaffolded` means model/table/route exists; `pending` means Scala business logic still needs manual porting.

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
| `linux` | `GeoLocationService` | not ported yet | `pending` |
| `linux` | `GroupDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `GroupInfoPrepareService` | not ported yet | `pending` |
| `linux` | `GroupListDao` | not ported yet | `pending` |
| `linux` | `GroupPermissionService` | not ported yet | `pending` |
| `linux` | `GroupService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `HSTSInterceptor` | not ported yet | `pending` |
| `linux` | `IgnoreListDao` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `ImageDao` | not ported yet | `pending` |
| `linux` | `ImageService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `IpBlockChecker` | not ported yet | `pending` |
| `linux` | `IpBlockDao` | not ported yet | `pending` |
| `linux` | `LastLoginInterceptor` | not ported yet | `pending` |
| `linux` | `LorCodeService` | not ported yet | `pending` |
| `linux` | `MemoriesDao` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `MessageTextService` | not ported yet | `pending` |
| `linux` | `MoreLikeThisService` | not ported yet | `pending` |
| `linux` | `MsgbaseDao` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `OpenSearchIndexCreationService` | not ported yet | `pending` |
| `linux` | `OpenSearchIndexService` | not ported yet | `pending` |
| `linux` | `Perf4jHandlerInterceptor` | not ported yet | `pending` |
| `linux` | `PollDao` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `PollPrepareService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `PreparedRemarkService` | not ported yet | `pending` |
| `linux` | `ProfileDao` | not ported yet | `pending` |
| `linux` | `ReactionDao` | not ported yet | `pending` |
| `linux` | `ReactionService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `RemarkDao` | not ported yet | `pending` |
| `linux` | `SameIpDao` | not ported yet | `pending` |
| `linux` | `SameIpService` | not ported yet | `pending` |
| `linux` | `ScoreUpdater` | not ported yet | `pending` |
| `linux` | `SearchService` | `src/routes/*`, `src/models.rs` | `ported` |
| `linux` | `SearchServiceRequest` | not ported yet | `pending` |
| `linux` | `SearchServiceResponse` | not ported yet | `pending` |
| `linux` | `SecretTokenService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
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
| `linux` | `UserLogDao` | not ported yet | `pending` |
| `linux` | `UserLogPrepareService` | not ported yet | `pending` |
| `linux` | `UserPermissionService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
| `linux` | `UserService` | not ported yet | `pending` |
| `linux` | `UserStatisticsService` | not ported yet | `pending` |
| `linux` | `UserTagDao` | not ported yet | `pending` |
| `linux` | `UserTagService` | not ported yet | `pending` |
| `linux` | `UserpicPermissionInterceptor` | not ported yet | `pending` |
| `linux` | `WarningDao` | not ported yet | `pending` |
| `linux` | `WarningService` | `src/models_compat.rs`, `db/migrations/0003_*`, route stub | `scaffolded` |
