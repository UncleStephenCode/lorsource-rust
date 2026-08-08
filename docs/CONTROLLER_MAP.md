# Current original controller and HTTP-surface inventory

> Generated from Java/Scala declarations. These counts are inventory data, not evidence of semantic parity with the Rust port.

- Spring handler methods: **179**
- Expanded Spring mapping variants: **193**
- Unique normalized MVC path templates: **131**
- Controllers with mapped handlers: **62**
- `@ResponseBody` mapping variants: **18**
- Bare method-level `@RequestMapping` variants: **4**
- Mapping variants with Spring regex path constraints: **7**
- Controller-wide `@ModelAttribute` providers: **9**
- Controller `@ExceptionHandler` methods: **18**

Declared effective methods: `ANY` 65, `GET` 39, `GET+HEAD` 17, `POST` 72

Non-MVC surface:

- WebSocket registrations: **1**
- URL rewrite/filter rules: **20**
- Spring resource handler patterns: **8**
- servlet URL mappings: **47**
- servlet filter mappings: **2**
- servlet error-page dispatches: **3**
- Spring MVC interceptors: **4**
- global controller-advice declarations: **1**
- default-servlet static roots/files: **17**

The detailed machine-readable contracts are in `docs/generated/current_java_routes.json` and `docs/generated/current_java_surface.json`.

| Controller | Handler methods | Expanded variants | Unique paths | Sources |
|---|---:|---:|---:|---|
| `AddCommentController` | 4 | 4 | 3 | `src/main/scala/ru/org/linux/comment/AddCommentController.scala` |
| `AddTopicController` | 4 | 4 | 2 | `src/main/scala/ru/org/linux/topic/AddTopicController.scala` |
| `ArchiveController` | 5 | 5 | 5 | `src/main/scala/ru/org/linux/topic/ArchiveController.scala` |
| `ArticlesBoxlet` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/boxlets/ArticlesBoxlet.scala` |
| `BanIPController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/auth/BanIPController.scala` |
| `DelIPController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/admin/DelIPController.scala` |
| `DeleteCommentController` | 4 | 4 | 2 | `src/main/scala/ru/org/linux/comment/DeleteCommentController.scala` |
| `DeleteImageController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/gallery/DeleteImageController.scala` |
| `DeleteTopicController` | 4 | 4 | 2 | `src/main/scala/ru/org/linux/topic/DeleteTopicController.scala` |
| `DeletedCommentController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/comment/DeletedCommentController.scala` |
| `DeregisterController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/DeregisterController.scala` |
| `EditCommentController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/comment/EditCommentController.scala` |
| `EditHistoryController` | 2 | 10 | 10 | `src/main/scala/ru/org/linux/edithistory/EditHistoryController.scala` |
| `EditProfileController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/EditProfileController.scala` |
| `EditRemarkController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/EditRemarkController.scala` |
| `EditSettingsController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/EditSettingsController.scala` |
| `EditTopicController` | 3 | 3 | 2 | `src/main/scala/ru/org/linux/topic/EditTopicController.scala` |
| `EmailDomainsBlockController` | 3 | 3 | 3 | `src/main/scala/ru/org/linux/admin/EmailDomainsBlockController.scala` |
| `ExceptionController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/exception/ExceptionController.scala` |
| `GalleryBoxlet` | 1 | 1 | 1 | `src/main/java/ru/org/linux/gallery/GalleryBoxlet.java` |
| `GeoLocationController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/auth/GeoLocationController.scala` |
| `GroupController` | 4 | 4 | 4 | `src/main/scala/ru/org/linux/group/GroupController.scala` |
| `GroupModificationController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/group/GroupModificationController.scala` |
| `HelpController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/help/HelpController.scala` |
| `HttpErrorController` | 2 | 2 | 2 | `src/main/scala/ru/org/linux/site/HttpErrorController.scala` |
| `LoginController` | 5 | 6 | 4 | `src/main/scala/ru/org/linux/auth/LoginController.scala` |
| `LostPasswordController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/LostPasswordController.scala` |
| `MainPageController` | 1 | 2 | 2 | `src/main/scala/ru/org/linux/spring/MainPageController.scala` |
| `MarkupPreviewController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/markup/MarkupPreviewController.scala` |
| `MemoriesController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/MemoriesController.scala` |
| `PollBoxlet` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/poll/PollBoxlet.scala` |
| `ReactionController` | 6 | 6 | 2 | `src/main/scala/ru/org/linux/reaction/ReactionController.scala` |
| `RegisterController` | 6 | 9 | 4 | `src/main/scala/ru/org/linux/user/RegisterController.scala` |
| `ResetPasswordController` | 3 | 3 | 2 | `src/main/scala/ru/org/linux/user/ResetPasswordController.scala` |
| `ResolveController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/topic/ResolveController.scala` |
| `SameIPController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/admin/SameIPController.scala` |
| `SearchControlController` | 3 | 3 | 1 | `src/main/scala/ru/org/linux/search/SearchControlController.scala` |
| `SearchController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/search/SearchController.scala` |
| `SectionController` | 2 | 2 | 2 | `src/main/scala/ru/org/linux/section/SectionController.scala` |
| `ServerInfoController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/spring/ServerInfoController.scala` |
| `ShowCommentsController` | 2 | 2 | 2 | `src/main/scala/ru/org/linux/comment/ShowCommentsController.scala` |
| `ShowRemarkController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/user/ShowRemarkController.scala` |
| `TagCloudBoxlet` | 1 | 1 | 1 | `src/main/java/ru/org/linux/boxlets/TagCloudBoxlet.java` |
| `TagController` | 8 | 8 | 5 | `src/main/scala/ru/org/linux/tag/TagController.scala` |
| `TagPageController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/tag/TagPageController.scala` |
| `TagTopicListController` | 2 | 2 | 2 | `src/main/scala/ru/org/linux/topic/TagTopicListController.scala` |
| `TopTenBoxlet` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/boxlets/TopTenBoxlet.scala` |
| `TopicController` | 5 | 5 | 5 | `src/main/scala/ru/org/linux/topic/TopicController.scala` |
| `TopicListController` | 6 | 6 | 6 | `src/main/scala/ru/org/linux/topic/TopicListController.scala` |
| `TopicModificationController` | 7 | 7 | 4 | `src/main/scala/ru/org/linux/topic/TopicModificationController.scala` |
| `TrackerController` | 2 | 2 | 2 | `src/main/scala/ru/org/linux/tracker/TrackerController.scala` |
| `UncommitedTopicsController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/topic/UncommitedTopicsController.scala` |
| `UserEventApiController` | 3 | 3 | 3 | `src/main/scala/ru/org/linux/user/UserEventApiController.scala` |
| `UserEventController` | 7 | 7 | 4 | `src/main/scala/ru/org/linux/user/UserEventController.scala` |
| `UserFilterController` | 11 | 11 | 4 | `src/main/scala/ru/org/linux/user/UserFilterController.scala` |
| `UserModificationController` | 12 | 12 | 3 | `src/main/scala/ru/org/linux/user/UserModificationController.scala` |
| `UserReactionsController` | 1 | 2 | 2 | `src/main/scala/ru/org/linux/reaction/UserReactionsController.scala` |
| `UserTopicListController` | 6 | 6 | 5 | `src/main/scala/ru/org/linux/topic/UserTopicListController.scala` |
| `UserpicController` | 2 | 2 | 1 | `src/main/scala/ru/org/linux/user/UserpicController.scala` |
| `VoteController` | 1 | 1 | 1 | `src/main/scala/ru/org/linux/poll/VoteController.scala` |
| `WarningController` | 3 | 3 | 2 | `src/main/scala/ru/org/linux/warning/WarningController.scala` |
| `WhoisController` | 3 | 3 | 2 | `src/main/scala/ru/org/linux/user/WhoisController.scala` |

## Interpretation limits

The extractor records declared mapping conditions (`params`, `headers`, `consumes`, `produces`), annotated parameters, bean-bindable form fields, `@ResponseBody`, literal view/model keys and direct `getParameter` calls. It does not execute Spring, follow the full service call graph, resolve security configuration, prove template equivalence or observe database/external-system side effects. Runtime differential tests remain required.
