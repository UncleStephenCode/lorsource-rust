# Original controller map

Controllers with mapped endpoints: **57**
Endpoint declarations: **184**

| Controller | Endpoints | Source | Main paths |
|---|---:|---|---|
| `AddCommentController` | 4 | `src/main/scala/ru/org/linux/comment/AddCommentController.scala` | `/add_comment.jsp`, `/add_comment.jsp`, `/add_comment_ajax`, `/comment-message.jsp` |
| `AddTopicController` | 4 | `src/main/scala/ru/org/linux/topic/AddTopicController.scala` | `/add-section.jsp`, `/add-section.jsp`, `/add.jsp`, `/add.jsp` |
| `ArchiveController` | 5 | `src/main/scala/ru/org/linux/topic/ArchiveController.scala` | `/articles/archive`, `/forum/{group}/archive`, `/gallery/archive`, `/news/archive`, `/polls/archive` |
| `ArticlesBoxlet` | 1 | `src/main/scala/ru/org/linux/boxlets/ArticlesBoxlet.scala` | `/articles.boxlet` |
| `BanIPController` | 1 | `src/main/scala/ru/org/linux/auth/BanIPController.scala` | `/banip.jsp` |
| `DelIPController` | 1 | `src/main/scala/ru/org/linux/admin/DelIPController.scala` | `/delip.jsp` |
| `DeleteCommentController` | 4 | `src/main/scala/ru/org/linux/comment/DeleteCommentController.scala` | `/delete_comment.jsp`, `/delete_comment.jsp`, `/undelete_comment`, `/undelete_comment` |
| `DeleteImageController` | 2 | `src/main/scala/ru/org/linux/gallery/DeleteImageController.scala` | `/delete_image`, `/delete_image` |
| `DeleteTopicController` | 4 | `src/main/scala/ru/org/linux/topic/DeleteTopicController.scala` | `/delete.jsp`, `/delete.jsp`, `/undelete`, `/undelete` |
| `DeletedCommentController` | 1 | `src/main/scala/ru/org/linux/comment/DeletedCommentController.scala` | `/view-deleted` |
| `DeregisterController` | 2 | `src/main/scala/ru/org/linux/user/DeregisterController.scala` | `/deregister.jsp`, `/deregister.jsp` |
| `EditCommentController` | 2 | `src/main/scala/ru/org/linux/comment/EditCommentController.scala` | `/edit_comment`, `/edit_comment` |
| `EditHistoryController` | 10 | `src/main/scala/ru/org/linux/edithistory/EditHistoryController.scala` | `/articles/{group}/{id}/history`, `/articles/{group}/{id}/{commentid}/history`, `/forum/{group}/{id}/history`, `/forum/{group}/{id}/{commentid}/history`, `/gallery/{group}/{id}/history`, … +5 |
| `EditProfileController` | 2 | `src/main/scala/ru/org/linux/user/EditProfileController.scala` | `/people/{nick}/edit`, `/people/{nick}/edit` |
| `EditRemarkController` | 2 | `src/main/scala/ru/org/linux/user/EditRemarkController.scala` | `/people/{nick}/remark`, `/people/{nick}/remark` |
| `EditSettingsController` | 2 | `src/main/scala/ru/org/linux/user/EditSettingsController.scala` | `/people/{nick}/settings`, `/people/{nick}/settings` |
| `EditTopicController` | 3 | `src/main/scala/ru/org/linux/topic/EditTopicController.scala` | `/commit.jsp`, `/edit.jsp`, `/edit.jsp` |
| `ExceptionController` | 1 | `src/main/scala/ru/org/linux/exception/ExceptionController.scala` | `/ExceptionResolver` |
| `GeoLocationController` | 1 | `src/main/scala/ru/org/linux/auth/GeoLocationController.scala` | `/admin/geoip` |
| `GroupController` | 4 | `src/main/scala/ru/org/linux/group/GroupController.scala` | `/forum/{group}`, `/forum/{group}/{year}/{month}`, `/group-lastmod.jsp`, `/group.jsp` |
| `GroupModificationController` | 2 | `src/main/scala/ru/org/linux/group/GroupModificationController.scala` | `/groupmod.jsp`, `/groupmod.jsp` |
| `HelpController` | 1 | `src/main/scala/ru/org/linux/help/HelpController.scala` | `/help/{page}` |
| `HttpErrorController` | 2 | `src/main/scala/ru/org/linux/site/HttpErrorController.scala` | `/errors/403`, `/errors/404` |
| `LoginController` | 6 | `src/main/scala/ru/org/linux/auth/LoginController.scala` | `/login.jsp`, `/login_process`, `/logout`, `/logout`, `/logout_all_sessions`, … +1 |
| `LostPasswordController` | 2 | `src/main/scala/ru/org/linux/user/LostPasswordController.scala` | `/lostpwd.jsp`, `/lostpwd.jsp` |
| `MainPageController` | 2 | `src/main/scala/ru/org/linux/spring/MainPageController.scala` | `/`, `/index.jsp` |
| `MarkupPreviewController` | 1 | `src/main/scala/ru/org/linux/markup/MarkupPreviewController.scala` | `/markup/preview` |
| `MemoriesController` | 2 | `src/main/scala/ru/org/linux/user/MemoriesController.scala` | `/memories.jsp`, `/memories.jsp` |
| `PollBoxlet` | 1 | `src/main/scala/ru/org/linux/poll/PollBoxlet.scala` | `/poll.boxlet` |
| `ReactionController` | 6 | `src/main/scala/ru/org/linux/reaction/ReactionController.scala` | `/reactions`, `/reactions`, `/reactions`, `/reactions`, `/reactions/ajax`, … +1 |
| `RegisterController` | 9 | `src/main/scala/ru/org/linux/user/RegisterController.scala` | `/activate`, `/activate`, `/activate`, `/activate.jsp`, `/activate.jsp`, … +4 |
| `ResetPasswordController` | 3 | `src/main/scala/ru/org/linux/user/ResetPasswordController.scala` | `/people/{nick}/profile`, `/reset-password`, `/reset-password` |
| `ResolveController` | 1 | `src/main/scala/ru/org/linux/topic/ResolveController.scala` | `/resolve.jsp` |
| `SameIPController` | 1 | `src/main/scala/ru/org/linux/admin/SameIPController.scala` | `/sameip.jsp` |
| `SearchControlController` | 3 | `src/main/scala/ru/org/linux/search/SearchControlController.scala` | `/admin/search-reindex`, `/admin/search-reindex`, `/admin/search-reindex` |
| `SearchController` | 1 | `src/main/scala/ru/org/linux/search/SearchController.scala` | `/search.jsp` |
| `SectionController` | 2 | `src/main/scala/ru/org/linux/section/SectionController.scala` | `/forum`, `/view-section.jsp` |
| `ServerInfoController` | 1 | `src/main/scala/ru/org/linux/spring/ServerInfoController.scala` | `/about` |
| `ShowCommentsController` | 2 | `src/main/scala/ru/org/linux/comment/ShowCommentsController.scala` | `/people/{nick}/deleted-comments`, `/show-comments.jsp` |
| `ShowRemarkController` | 1 | `src/main/scala/ru/org/linux/user/ShowRemarkController.scala` | `/people/{nick}/remarks` |
| `TagController` | 8 | `src/main/scala/ru/org/linux/tag/TagController.scala` | `/tags`, `/tags`, `/tags.jsp`, `/tags/change`, `/tags/change`, … +3 |
| `TagPageController` | 1 | `src/main/scala/ru/org/linux/tag/TagPageController.scala` | `/tag/{tag}` |
| `TagTopicListController` | 2 | `src/main/scala/ru/org/linux/topic/TagTopicListController.scala` | `/tag/{tag}`, `/view-news.jsp` |
| `TopTenBoxlet` | 1 | `src/main/scala/ru/org/linux/boxlets/TopTenBoxlet.scala` | `/top10.boxlet` |
| `TopicController` | 5 | `src/main/scala/ru/org/linux/topic/TopicController.scala` | `/jump-message.jsp`, `/view-message.jsp`, `/{section}/{group}/{id}`, `/{section}/{group}/{id}/page{page}`, `/{section}/{group}/{id}/thread/{threadRoot}` |
| `TopicListController` | 6 | `src/main/scala/ru/org/linux/topic/TopicListController.scala` | `/forum/lenta`, `/section-rss.jsp`, `/show-topics.jsp`, `/{section}/`, `/{section}/archive/{year}/{month}`, … +1 |
| `TopicModificationController` | 7 | `src/main/scala/ru/org/linux/topic/TopicModificationController.scala` | `/mt.jsp`, `/mt.jsp`, `/mtn.jsp`, `/setpostscore.jsp`, `/setpostscore.jsp`, … +2 |
| `TrackerController` | 2 | `src/main/scala/ru/org/linux/tracker/TrackerController.scala` | `/tracker`, `/tracker.jsp` |
| `UserEventApiController` | 3 | `src/main/scala/ru/org/linux/user/UserEventApiController.scala` | `/notifications-count`, `/notifications-reset`, `/yandex-tableau` |
| `UserEventController` | 7 | `src/main/scala/ru/org/linux/user/UserEventController.scala` | `/notifications`, `/notifications`, `/notifications-click`, `/notifications-click/ajax`, `/show-replies.jsp`, … +2 |
| `UserFilterController` | 11 | `src/main/scala/ru/org/linux/user/UserFilterController.scala` | `/user-filter`, `/user-filter/favorite-tag`, `/user-filter/favorite-tag`, `/user-filter/favorite-tag`, `/user-filter/favorite-tag`, … +6 |
| `UserModificationController` | 12 | `src/main/scala/ru/org/linux/user/UserModificationController.scala` | `/people/{nick}/profile/wipe`, `/remove-userpic.jsp`, `/usermod.jsp`, `/usermod.jsp`, `/usermod.jsp`, … +7 |
| `UserTopicListController` | 5 | `src/main/scala/ru/org/linux/topic/UserTopicListController.scala` | `/people/{nick}`, `/people/{nick}/deleted-topics`, `/people/{nick}/drafts`, `/people/{nick}/favs`, `/people/{nick}/tracked` |
| `UserpicController` | 2 | `src/main/scala/ru/org/linux/user/UserpicController.scala` | `/addphoto.jsp`, `/addphoto.jsp` |
| `VoteController` | 1 | `src/main/scala/ru/org/linux/poll/VoteController.scala` | `/vote.jsp` |
| `WarningController` | 3 | `src/main/scala/ru/org/linux/warning/WarningController.scala` | `/clear-warning`, `/post-warning`, `/post-warning` |
| `WhoisController` | 3 | `src/main/scala/ru/org/linux/user/WhoisController.scala` | `/people/{nick}/profile`, `/people/{nick}/profile`, `/whois.jsp` |
