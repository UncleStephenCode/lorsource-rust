# Structural route declaration comparison

> This is not a semantic parity report. It compares normalized path templates and declared methods only. It does not verify request parameters, headers/content negotiation, authentication/authorization, status/redirects, HTML, database changes or side effects.

Expanded original Spring mapping variants: **193**
Path and all declared methods present in a Rust declaration: **174**
Path present with only partial/unrestricted-method overlap: **19**
Path exists but method differs: **0**
Missing route declaration: **0**

Spring `ANY` mappings are intentionally reported as partial unless the Rust inventory also declares `ANY`. Extra Rust methods and Axum's runtime HEAD behavior are not evaluated here.

| Structural status | Methods | Original path | Mapping conditions | Controller.handler | Rust declaration |
|---|---|---|---|---|---|
| method-declared | `ANY` | `/` | `` | `MainPageController.mainPage` | `ANY /` |
| method-declared | `ANY` | `/ExceptionResolver` | `` | `ExceptionController.defaultExceptionHandler` | `ANY /ExceptionResolver` |
| method-declared | `ANY` | `/about` | `` | `ServerInfoController.serverInfo` | `ANY /about` |
| method-declared | `GET` | `/activate` | `` | `RegisterController.activateForm` | `GET,POST /activate` |
| method-declared | `POST` | `/activate` | `params=action` | `RegisterController.activateNew` | `GET,POST /activate` |
| method-declared | `POST` | `/activate` | `params=!action` | `RegisterController.activate` | `GET,POST /activate` |
| method-declared | `GET` | `/activate.jsp` | `` | `RegisterController.activateForm` | `GET,POST /activate.jsp` |
| method-declared | `POST` | `/activate.jsp` | `params=action` | `RegisterController.activateNew` | `GET,POST /activate.jsp` |
| method-declared | `POST` | `/activate.jsp` | `params=!action` | `RegisterController.activate` | `GET,POST /activate.jsp` |
| method-declared | `ANY` | `/add-section.jsp` | `params=section` | `AddTopicController.showFormWithSection` | `ANY /add-section.jsp` |
| method-declared | `ANY` | `/add-section.jsp` | `params=!section` | `AddTopicController.showFormAllSections` | `ANY /add-section.jsp` |
| method-declared | `GET` | `/add.jsp` | `` | `AddTopicController.add` | `GET,POST /add.jsp` |
| method-declared | `POST` | `/add.jsp` | `` | `AddTopicController.doAdd` | `GET,POST /add.jsp` |
| method-declared | `GET` | `/add_comment.jsp` | `` | `AddCommentController.showFormReply` | `GET,POST /add_comment.jsp` |
| method-declared | `POST` | `/add_comment.jsp` | `` | `AddCommentController.addComment` | `GET,POST /add_comment.jsp` |
| method-declared | `POST` | `/add_comment_ajax` | `produces=application/json; charset=UTF-8` | `AddCommentController.addCommentAjax` | `POST /add_comment_ajax` |
| method-declared | `GET` | `/addphoto.jsp` | `` | `UserpicController.showForm` | `GET,POST /addphoto.jsp` |
| method-declared | `POST` | `/addphoto.jsp` | `` | `UserpicController.addPhoto` | `GET,POST /addphoto.jsp` |
| method-declared | `ANY` | `/admin/email-domains` | `` | `EmailDomainsBlockController.list` | `ANY /admin/email-domains` |
| method-declared | `POST` | `/admin/email-domains/add` | `` | `EmailDomainsBlockController.add` | `POST /admin/email-domains/add` |
| method-declared | `POST` | `/admin/email-domains/delete` | `` | `EmailDomainsBlockController.delete` | `POST /admin/email-domains/delete` |
| method-declared | `GET` | `/admin/geoip` | `` | `GeoLocationController.geoip` | `GET /admin/geoip` |
| method-declared | `POST` | `/admin/search-reindex` | `params=action=all` | `SearchControlController.reindexAll` | `GET,POST /admin/search-reindex` |
| method-declared | `POST` | `/admin/search-reindex` | `params=action=current` | `SearchControlController.reindexCurrentMonth` | `GET,POST /admin/search-reindex` |
| method-declared | `GET` | `/admin/search-reindex` | `` | `SearchControlController.reindex` | `GET,POST /admin/search-reindex` |
| method-declared | `ANY` | `/articles.boxlet` | `` | `ArticlesBoxlet.getData` | `ANY /articles.boxlet` |
| method-declared | `ANY` | `/articles/archive` | `` | `ArchiveController.articlesArchive` | `ANY /articles/archive; ANY /articles/archive/` |
| method-declared | `ANY` | `/articles/{group}/{id}/history` | `` | `EditHistoryController.showEditInfo` | `ANY /articles/{group}/{id}/history` |
| method-declared | `ANY` | `/articles/{group}/{id}/{commentid}/history` | `` | `EditHistoryController.showCommentEditInfo` | `ANY /articles/{group}/{id}/{commentid}/history` |
| method-declared | `POST` | `/banip.jsp` | `` | `BanIPController.banIP` | `POST /banip.jsp` |
| method-declared | `ANY` | `/check-login` | `` | `RegisterController.ajaxLoginCheck` | `ANY /check-login` |
| method-declared | `POST` | `/clear-warning` | `` | `WarningController.clear` | `POST /clear-warning` |
| partial-method | `ANY` | `/comment-message.jsp` | `` | `AddCommentController.showFormTopic` | `GET,POST,PUT,DELETE,PATCH /comment-message.jsp` |
| method-declared | `GET` | `/commit.jsp` | `` | `EditTopicController.showCommitForm` | `GET /commit.jsp` |
| method-declared | `GET` | `/delete.jsp` | `` | `DeleteTopicController.showForm` | `GET,POST /delete.jsp` |
| method-declared | `POST` | `/delete.jsp` | `` | `DeleteTopicController.deleteMessage` | `GET,POST /delete.jsp` |
| method-declared | `GET` | `/delete_comment.jsp` | `` | `DeleteCommentController.showForm` | `GET,POST /delete_comment.jsp` |
| method-declared | `POST` | `/delete_comment.jsp` | `` | `DeleteCommentController.deleteComments` | `GET,POST /delete_comment.jsp` |
| method-declared | `GET` | `/delete_image` | `` | `DeleteImageController.deleteForm` | `GET,POST /delete_image` |
| method-declared | `POST` | `/delete_image` | `` | `DeleteImageController.deleteImage` | `GET,POST /delete_image` |
| method-declared | `POST` | `/delip.jsp` | `` | `DelIPController.delIp` | `POST /delip.jsp` |
| partial-method | `GET,HEAD` | `/deregister.jsp` | `` | `DeregisterController.show` | `GET,POST /deregister.jsp` |
| method-declared | `POST` | `/deregister.jsp` | `` | `DeregisterController.deregister` | `GET,POST /deregister.jsp` |
| method-declared | `GET` | `/edit.jsp` | `` | `EditTopicController.showEditForm` | `GET,POST /edit.jsp` |
| method-declared | `POST` | `/edit.jsp` | `` | `EditTopicController.edit` | `GET,POST /edit.jsp` |
| method-declared | `GET` | `/edit_comment` | `` | `EditCommentController.editCommentShowHandler` | `GET,POST /edit_comment` |
| method-declared | `POST` | `/edit_comment` | `` | `EditCommentController.editCommentPostHandler` | `GET,POST /edit_comment` |
| method-declared | `ANY` | `/errors/403` | `` | `HttpErrorController.handle403` | `ANY /errors/403` |
| method-declared | `ANY` | `/errors/404` | `` | `HttpErrorController.handle404` | `ANY /errors/404` |
| method-declared | `ANY` | `/forum` | `` | `SectionController.forum` | `ANY /forum; ANY /forum/` |
| method-declared | `ANY` | `/forum/lenta` | `` | `TopicListController.forum` | `ANY /forum/lenta` |
| method-declared | `ANY` | `/forum/{group}` | `` | `GroupController.forum` | `ANY /forum/{group}` |
| method-declared | `ANY` | `/forum/{group}/archive` | `` | `ArchiveController.forumArchive` | `ANY /forum/{group}/archive; ANY /forum/{group}/archive/` |
| method-declared | `ANY` | `/forum/{group}/{id}/history` | `` | `EditHistoryController.showEditInfo` | `ANY /forum/{group}/{id}/history` |
| method-declared | `ANY` | `/forum/{group}/{id}/{commentid}/history` | `` | `EditHistoryController.showCommentEditInfo` | `ANY /forum/{group}/{id}/{commentid}/history` |
| method-declared | `ANY` | `/forum/{group}/{year}/{month}` | `` | `GroupController.forumArchive` | `ANY /forum/{group}/{id_or_year}/{page_or_month}; ANY /forum/{group}/{id_or_year}/{page_or_month}/` |
| method-declared | `ANY` | `/gallery.boxlet` | `` | `GalleryBoxlet.getData` | `ANY /gallery.boxlet` |
| method-declared | `ANY` | `/gallery/archive` | `` | `ArchiveController.galleryArchive` | `ANY /gallery/archive; ANY /gallery/archive/` |
| method-declared | `ANY` | `/gallery/{group}/{id}/history` | `` | `EditHistoryController.showEditInfo` | `ANY /gallery/{group}/{id}/history` |
| method-declared | `ANY` | `/gallery/{group}/{id}/{commentid}/history` | `` | `EditHistoryController.showCommentEditInfo` | `ANY /gallery/{group}/{id}/{commentid}/history` |
| method-declared | `ANY` | `/group-lastmod.jsp` | `` | `GroupController.topicsLastmod` | `ANY /group-lastmod.jsp` |
| method-declared | `ANY` | `/group.jsp` | `` | `GroupController.topics` | `ANY /group.jsp` |
| method-declared | `GET` | `/groupmod.jsp` | `` | `GroupModificationController.showForm` | `GET,POST /groupmod.jsp` |
| method-declared | `POST` | `/groupmod.jsp` | `` | `GroupModificationController.modifyGroup` | `GET,POST /groupmod.jsp` |
| method-declared | `ANY` | `/help/{page}` | `` | `HelpController.helpPage` | `ANY /help/{page}` |
| method-declared | `ANY` | `/index.jsp` | `` | `MainPageController.mainPage` | `ANY /index.jsp` |
| partial-method | `GET,HEAD` | `/jump-message.jsp` | `` | `TopicController.jumpMessage` | `GET /jump-message.jsp` |
| partial-method | `GET,HEAD` | `/login.jsp` | `` | `LoginController.loginForm` | `GET /login.jsp` |
| method-declared | `POST` | `/login_process` | `` | `LoginController.loginProcess` | `POST /login_process` |
| method-declared | `POST` | `/logout` | `` | `LoginController.logout` | `GET,POST /logout` |
| method-declared | `GET` | `/logout` | `` | `LoginController.logoutLink` | `GET,POST /logout` |
| method-declared | `POST` | `/logout_all_sessions` | `` | `LoginController.logoutAllDevices` | `GET,POST /logout_all_sessions` |
| method-declared | `GET` | `/logout_all_sessions` | `` | `LoginController.logoutLink` | `GET,POST /logout_all_sessions` |
| method-declared | `GET` | `/lostpwd.jsp` | `` | `LostPasswordController.showForm` | `GET,POST /lostpwd.jsp` |
| method-declared | `POST` | `/lostpwd.jsp` | `` | `LostPasswordController.sendPassword` | `GET,POST /lostpwd.jsp` |
| method-declared | `POST` | `/markup/preview` | `produces=application/json; charset=UTF-8` | `MarkupPreviewController.preview` | `POST /markup/preview` |
| method-declared | `POST` | `/memories.jsp` | `params=add` | `MemoriesController.add` | `POST /memories.jsp` |
| method-declared | `POST` | `/memories.jsp` | `params=remove` | `MemoriesController.remove` | `POST /memories.jsp` |
| method-declared | `POST` | `/mt.jsp` | `` | `TopicModificationController.moveTopic` | `GET,POST /mt.jsp` |
| method-declared | `GET` | `/mt.jsp` | `` | `TopicModificationController.moveToForumForm` | `GET,POST /mt.jsp` |
| method-declared | `GET` | `/mtn.jsp` | `` | `TopicModificationController.movePremoderatedForm` | `GET /mtn.jsp` |
| method-declared | `ANY` | `/news/archive` | `` | `ArchiveController.newsArchive` | `ANY /news/archive; ANY /news/archive/` |
| method-declared | `ANY` | `/news/{group}/{id}/history` | `` | `EditHistoryController.showEditInfo` | `ANY /news/{group}/{id}/history` |
| method-declared | `ANY` | `/news/{group}/{id}/{commentid}/history` | `` | `EditHistoryController.showCommentEditInfo` | `ANY /news/{group}/{id}/{commentid}/history` |
| method-declared | `POST` | `/notifications` | `` | `UserEventController.resetNotifications` | `GET,POST /notifications` |
| partial-method | `GET,HEAD` | `/notifications` | `` | `UserEventController.showNotifications` | `GET,POST /notifications` |
| method-declared | `POST` | `/notifications-click` | `` | `UserEventController.clickNotifications` | `POST /notifications-click` |
| method-declared | `POST` | `/notifications-click/ajax` | `produces=application/json` | `UserEventController.clickNotificationsAjax` | `POST /notifications-click/ajax` |
| method-declared | `GET` | `/notifications-count` | `` | `UserEventApiController.getEventsCount` | `GET /notifications-count` |
| method-declared | `POST` | `/notifications-reset` | `` | `UserEventApiController.resetNotifications` | `POST /notifications-reset` |
| method-declared | `ANY` | `/people/{nick}` | `params=output=rss` | `UserTopicListController.showUserTopicsRssGone` | `ANY /people/{nick}; ANY /people/{nick}/` |
| method-declared | `ANY` | `/people/{nick}` | `` | `UserTopicListController.showUserTopics` | `ANY /people/{nick}; ANY /people/{nick}/` |
| method-declared | `ANY` | `/people/{nick}/deleted-comments` | `` | `ShowCommentsController.showDeletedComments` | `ANY /people/{nick}/deleted-comments` |
| method-declared | `GET` | `/people/{nick}/deleted-topics` | `` | `UserTopicListController.showDeletedTopics` | `GET /people/{nick}/deleted-topics` |
| method-declared | `ANY` | `/people/{nick}/drafts` | `` | `UserTopicListController.showUserDrafts` | `ANY /people/{nick}/drafts` |
| method-declared | `GET` | `/people/{nick}/edit` | `` | `EditProfileController.show` | `GET,POST /people/{nick}/edit` |
| method-declared | `POST` | `/people/{nick}/edit` | `` | `EditProfileController.edit` | `GET,POST /people/{nick}/edit` |
| method-declared | `ANY` | `/people/{nick}/favs` | `` | `UserTopicListController.showUserFavs` | `ANY /people/{nick}/favs` |
| partial-method | `GET,HEAD` | `/people/{nick}/profile` | `params=reset-password` | `ResetPasswordController.showModeratorForm` | `GET /people/{nick}/profile; GET /people/{nick}/profile/` |
| partial-method | `GET,HEAD` | `/people/{nick}/profile` | `` | `WhoisController.getInfoNew` | `GET /people/{nick}/profile; GET /people/{nick}/profile/` |
| partial-method | `GET,HEAD` | `/people/{nick}/profile` | `params=year-stats` | `WhoisController.yearStats` | `GET /people/{nick}/profile; GET /people/{nick}/profile/` |
| partial-method | `GET,HEAD` | `/people/{nick}/profile/wipe` | `` | `UserModificationController.wipe` | `GET /people/{nick}/profile/wipe` |
| method-declared | `ANY` | `/people/{nick}/reactions` | `` | `UserReactionsController.reactions` | `ANY /people/{nick}/reactions` |
| method-declared | `ANY` | `/people/{nick}/reactions/{mode}` | `` | `UserReactionsController.reactions` | `ANY /people/{nick}/reactions/{mode}` |
| method-declared | `GET` | `/people/{nick}/remark` | `` | `EditRemarkController.showForm` | `GET,POST /people/{nick}/remark; GET,POST /people/{nick}/remark/` |
| method-declared | `POST` | `/people/{nick}/remark` | `` | `EditRemarkController.editProfile` | `GET,POST /people/{nick}/remark; GET,POST /people/{nick}/remark/` |
| method-declared | `ANY` | `/people/{nick}/remarks` | `` | `ShowRemarkController.showRemarks` | `ANY /people/{nick}/remarks` |
| method-declared | `GET` | `/people/{nick}/settings` | `` | `EditSettingsController.showForm` | `GET,POST /people/{nick}/settings; GET,POST /people/{nick}/settings/` |
| method-declared | `POST` | `/people/{nick}/settings` | `` | `EditSettingsController.updateSettings` | `GET,POST /people/{nick}/settings; GET,POST /people/{nick}/settings/` |
| method-declared | `ANY` | `/people/{nick}/tracked` | `` | `UserTopicListController.showUserWatches` | `ANY /people/{nick}/tracked` |
| method-declared | `ANY` | `/poll.boxlet` | `` | `PollBoxlet.getData` | `ANY /poll.boxlet` |
| method-declared | `ANY` | `/polls/archive` | `` | `ArchiveController.pollsArchive` | `ANY /polls/archive; ANY /polls/archive/` |
| method-declared | `ANY` | `/polls/{group}/{id}/history` | `` | `EditHistoryController.showEditInfo` | `ANY /polls/{group}/{id}/history` |
| method-declared | `ANY` | `/polls/{group}/{id}/{commentid}/history` | `` | `EditHistoryController.showCommentEditInfo` | `ANY /polls/{group}/{id}/{commentid}/history` |
| method-declared | `GET` | `/post-warning` | `` | `WarningController.showForm` | `GET,POST /post-warning` |
| method-declared | `POST` | `/post-warning` | `` | `WarningController.post` | `GET,POST /post-warning` |
| method-declared | `GET` | `/reactions` | `params=comment` | `ReactionController.commentReaction` | `GET,POST /reactions` |
| method-declared | `POST` | `/reactions` | `params=comment` | `ReactionController.setCommentReaction` | `GET,POST /reactions` |
| method-declared | `GET` | `/reactions` | `params=!comment` | `ReactionController.topicReaction` | `GET,POST /reactions` |
| method-declared | `POST` | `/reactions` | `params=!comment` | `ReactionController.setTopicReaction` | `GET,POST /reactions` |
| method-declared | `POST` | `/reactions/ajax` | `params=comment` | `ReactionController.setCommentReactionAjax` | `POST /reactions/ajax` |
| method-declared | `POST` | `/reactions/ajax` | `params=!comment` | `ReactionController.setTopicReactionAjax` | `POST /reactions/ajax` |
| method-declared | `GET` | `/register.jsp` | `` | `RegisterController.register` | `GET,POST /register.jsp` |
| method-declared | `POST` | `/register.jsp` | `` | `RegisterController.doRegister` | `GET,POST /register.jsp` |
| method-declared | `POST` | `/remove-userpic.jsp` | `` | `UserModificationController.removeUserpic` | `POST /remove-userpic.jsp` |
| method-declared | `GET` | `/reset-password` | `` | `ResetPasswordController.showCodeForm` | `GET,POST /reset-password` |
| method-declared | `POST` | `/reset-password` | `` | `ResetPasswordController.resetPassword` | `GET,POST /reset-password` |
| partial-method | `ANY` | `/resolve.jsp` | `` | `ResolveController.resolve` | `GET,POST,PUT,DELETE,PATCH /resolve.jsp` |
| method-declared | `ANY` | `/sameip.jsp` | `` | `SameIPController.sameIP` | `ANY /sameip.jsp` |
| partial-method | `GET,HEAD` | `/search.jsp` | `` | `SearchController.search` | `GET /search.jsp` |
| method-declared | `ANY` | `/section-rss.jsp` | `` | `TopicListController.showRSS` | `ANY /section-rss.jsp` |
| method-declared | `GET` | `/setpostscore.jsp` | `` | `TopicModificationController.showForm` | `GET,POST /setpostscore.jsp` |
| method-declared | `POST` | `/setpostscore.jsp` | `` | `TopicModificationController.modifyTopic` | `GET,POST /setpostscore.jsp` |
| method-declared | `ANY` | `/show-comments.jsp` | `` | `ShowCommentsController.showComments` | `ANY /show-comments.jsp` |
| partial-method | `GET,HEAD` | `/show-replies.jsp` | `params=!output,!nick` | `UserEventController.showNotificationsOld` | `GET /show-replies.jsp` |
| partial-method | `GET,HEAD` | `/show-replies.jsp` | `params=!output` | `UserEventController.showNotificationsForModerator` | `GET /show-replies.jsp` |
| partial-method | `GET,HEAD` | `/show-replies.jsp` | `params=output` | `UserEventController.repliesFeed` | `GET /show-replies.jsp` |
| method-declared | `GET` | `/show-topics.jsp` | `` | `TopicListController.showUserTopics` | `GET /show-topics.jsp` |
| partial-method | `GET,HEAD` | `/tag/{tag}` | `params=!section` | `TagPageController.tagPage` | `GET /tag/{tag}` |
| partial-method | `GET,HEAD` | `/tag/{tag}` | `params=section` | `TagTopicListController.tagFeed` | `GET /tag/{tag}` |
| method-declared | `ANY` | `/tagcloud.boxlet` | `` | `TagCloudBoxlet.getData` | `ANY /tagcloud.boxlet` |
| method-declared | `ANY` | `/tags` | `` | `TagController.showDefaultTagListHandlertags` | `ANY /tags` |
| method-declared | `ANY` | `/tags` | `params=term` | `TagController.showTagListHandlerJSON` | `ANY /tags` |
| method-declared | `ANY` | `/tags.jsp` | `` | `TagController.oldTagsRedirectHandler` | `ANY /tags.jsp` |
| method-declared | `GET` | `/tags/change` | `` | `TagController.changeTagShowFormHandler` | `GET,POST /tags/change` |
| method-declared | `POST` | `/tags/change` | `` | `TagController.changeTagSubmitHandler` | `GET,POST /tags/change` |
| method-declared | `GET` | `/tags/delete` | `` | `TagController.deleteTagShowFormHandler` | `GET,POST /tags/delete` |
| method-declared | `POST` | `/tags/delete` | `` | `TagController.deleteTagSubmitHandler` | `GET,POST /tags/delete` |
| method-declared | `ANY` | `/tags/{firstLetter}` | `` | `TagController.showTagListHandler` | `ANY /tags/{first_letter}` |
| method-declared | `ANY` | `/top10.boxlet` | `` | `TopTenBoxlet.getData` | `ANY /top10.boxlet` |
| method-declared | `ANY` | `/tracker` | `` | `TrackerController.tracker` | `ANY /tracker; ANY /tracker/` |
| method-declared | `ANY` | `/tracker.jsp` | `` | `TrackerController.trackerOldUrl` | `ANY /tracker.jsp` |
| method-declared | `GET` | `/uncommit.jsp` | `` | `TopicModificationController.uncommitForm` | `GET,POST /uncommit.jsp` |
| method-declared | `POST` | `/uncommit.jsp` | `` | `TopicModificationController.uncommit` | `GET,POST /uncommit.jsp` |
| method-declared | `GET` | `/undelete` | `` | `DeleteTopicController.undeleteForm` | `GET,POST /undelete` |
| method-declared | `POST` | `/undelete` | `` | `DeleteTopicController.undelete` | `GET,POST /undelete` |
| method-declared | `GET` | `/undelete_comment` | `` | `DeleteCommentController.showUndeleteForm` | `GET,POST /undelete_comment` |
| method-declared | `POST` | `/undelete_comment` | `` | `DeleteCommentController.undelete` | `GET,POST /undelete_comment` |
| partial-method | `GET,HEAD` | `/user-filter` | `` | `UserFilterController.showList` | `GET /user-filter` |
| method-declared | `POST` | `/user-filter/favorite-tag` | `params=add` | `UserFilterController.favoriteTagAddHTML` | `POST /user-filter/favorite-tag` |
| method-declared | `POST` | `/user-filter/favorite-tag` | `params=add; headers=Accept=application/json` | `UserFilterController.favoriteTagAddJSON` | `POST /user-filter/favorite-tag` |
| method-declared | `POST` | `/user-filter/favorite-tag` | `params=del` | `UserFilterController.favoriteTagDel` | `POST /user-filter/favorite-tag` |
| method-declared | `POST` | `/user-filter/favorite-tag` | `params=del; headers=Accept=application/json` | `UserFilterController.favoriteTagDelJSON` | `POST /user-filter/favorite-tag` |
| method-declared | `POST` | `/user-filter/ignore-tag` | `params=add` | `UserFilterController.ignoreTagAdd` | `POST /user-filter/ignore-tag` |
| method-declared | `POST` | `/user-filter/ignore-tag` | `params=add; headers=Accept=application/json` | `UserFilterController.ignoreTagAddJSON` | `POST /user-filter/ignore-tag` |
| method-declared | `POST` | `/user-filter/ignore-tag` | `params=del` | `UserFilterController.ignoreTagDel` | `POST /user-filter/ignore-tag` |
| method-declared | `POST` | `/user-filter/ignore-tag` | `params=del; headers=Accept=application/json` | `UserFilterController.ignoreTagDelJSON` | `POST /user-filter/ignore-tag` |
| method-declared | `POST` | `/user-filter/ignore-user` | `params=add` | `UserFilterController.listAdd` | `POST /user-filter/ignore-user` |
| method-declared | `POST` | `/user-filter/ignore-user` | `params=del` | `UserFilterController.listDel` | `POST /user-filter/ignore-user` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=block` | `UserModificationController.blockUser` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=score50` | `UserModificationController.score50` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=unblock` | `UserModificationController.unblockUser` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=block-n-delete-comments` | `UserModificationController.blockAndMassiveDeleteCommentUser` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=toggle_corrector` | `UserModificationController.toggleUserCorrector` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=reset-password` | `UserModificationController.resetPassword` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=remove_userinfo` | `UserModificationController.removeUserInfo` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=remove_town` | `UserModificationController.removeTown` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=remove_url` | `UserModificationController.removeUrl` | `GET,POST /usermod.jsp` |
| method-declared | `POST` | `/usermod.jsp` | `params=action=freeze` | `UserModificationController.freezeUser` | `GET,POST /usermod.jsp` |
| partial-method | `GET,HEAD` | `/view-all.jsp` | `` | `UncommitedTopicsController.viewAll` | `GET /view-all.jsp` |
| method-declared | `ANY` | `/view-deleted` | `` | `DeletedCommentController.viewDeleted` | `ANY /view-deleted` |
| method-declared | `ANY` | `/view-message.jsp` | `` | `TopicController.getMessageOld` | `ANY /view-message.jsp` |
| partial-method | `GET,HEAD` | `/view-news.jsp` | `params=tag` | `TagTopicListController.tagFeedOld` | `GET /view-news.jsp` |
| method-declared | `ANY` | `/view-section.jsp` | `` | `SectionController.oldLink` | `ANY /view-section.jsp` |
| method-declared | `POST` | `/vote.jsp` | `` | `VoteController.vote` | `POST /vote.jsp` |
| method-declared | `ANY` | `/whois.jsp` | `` | `WhoisController.getInfo` | `ANY /whois.jsp` |
| method-declared | `GET` | `/yandex-tableau` | `produces=application/json` | `UserEventApiController.getYandexWidget` | `GET /yandex-tableau` |
| method-declared | `ANY` | `/{section}/` | `` | `TopicListController.topics` | `ANY /forum; ANY /forum/; ANY /news/; ANY /polls/; ANY /articles/; ANY /gallery/` |
| method-declared | `ANY` | `/{section}/archive/{year}/{month}` | `` | `TopicListController.sectionArchive` | `ANY /news/archive/{year}/{month}; ANY /news/archive/{year}/{month}/; ANY /polls/archive/{year}/{month}; ANY /polls/archive/{year}/{month}/; ANY /articles/archive/{year}/{month}; ANY /articles/archive/{year}/{month}/; ANY /gallery/archive/{year}/{month}; ANY /gallery/archive/{year}/{month}/` |
| method-declared | `ANY` | `/{section}/{group}` | `` | `TopicListController.topicsByGroup` | `ANY /forum/{group}; ANY /news/{group}; ANY /polls/{group}; ANY /articles/{group}; ANY /gallery/{group}` |
| method-declared | `ANY` | `/{section}/{group}/{id}` | `` | `TopicController.getMessageMain` | `ANY /forum/{group}/{id}; ANY /news/{group}/{id}; ANY /polls/{group}/{id}; ANY /articles/{group}/{id}; ANY /gallery/{group}/{id}` |
| method-declared | `GET` | `/{section}/{group}/{id}/page{page}` | `` | `TopicController.getMessagePage` | `ANY /forum/{group}/{id_or_year}/{page_or_month}; ANY /forum/{group}/{id_or_year}/{page_or_month}/; GET /news/{group}/{id}/{page_marker}; GET /polls/{group}/{id}/{page_marker}; GET /articles/{group}/{id}/{page_marker}; GET /gallery/{group}/{id}/{page_marker}` |
| method-declared | `GET` | `/{section}/{group}/{id}/thread/{threadRoot}` | `` | `TopicController.getMessageThread` | `GET /forum/{group}/{id}/thread/{thread_root}; GET /news/{group}/{id}/thread/{thread_root}; GET /polls/{group}/{id}/thread/{thread_root}; GET /articles/{group}/{id}/thread/{thread_root}; GET /gallery/{group}/{id}/thread/{thread_root}` |
