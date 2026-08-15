use crate::{
    application::boxlet::CBoxletService,
    auth::CurrentUser,
    domain::boxlet::model::{StPollBoxlet, StTopicBoxletItem},
    error::{AppError, Result},
    infra::postgres::boxlet_repository::CBoxletPgRepository,
    markup,
    models::{CommentItem, TopicDetail, UserSummary},
    state::AppState,
};
use askama::Template;
use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;

#[derive(Template)]
#[template(path = "notifications.html")]
struct StNotificationsTemplate {
    sContentHtml: String,
    sNick: String,
}

/// Maps UserEventFilterEnum's `getName` (lowercase enum case) to its `dbType`.
pub(crate) fn filter_db_type(filter: &str) -> Option<&'static str> {
    match filter {
        "answers" => Some("REPLY"),
        "favorites" => Some("WATCH"),
        "deleted" => Some("DEL"),
        "reference" => Some("REF"),
        "tag" => Some("TAG"),
        "reaction" => Some("REACTION"),
        "warning" => Some("WARNING"),
        _ => None, // "all" or unrecognized
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct NotificationEvent {
    pub id: i32,
    pub event_date: chrono::DateTime<chrono::Utc>,
    pub subj: String,
    pub msgid: i32,
    pub cid: Option<i32>,
    pub unread: bool,
    pub event_type: String,
    pub section_prefix: String,
    pub section_name: String,
    pub group_urlname: String,
    pub origin_nick: Option<String>,
    pub author_nick: String,
    pub event_message: Option<String>,
    pub closed_warning: bool,
    pub bonus: Option<i32>,
    pub tags: Vec<String>,
    pub message_text: String,
    pub message_markup: String,
    /// Reaction currently stored on the target. `None` means that an old,
    /// already-read event outlived a subsequently removed reaction.
    pub reaction: Option<String>,
}

impl NotificationEvent {
    pub(crate) fn sSubjectPlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.subj)
    }

    pub(crate) fn link(&self) -> String {
        if self.event_type == "DEL"
            && let Some(iCommentId) = self.cid
        {
            return format!("/view-deleted?id={iCommentId}#comment-{iCommentId}");
        }
        let anchor = self.cid.map(|id| format!("?cid={id}")).unwrap_or_default();
        format!(
            "/{}/{}/{}{anchor}",
            self.section_prefix, self.group_urlname, self.msgid
        )
    }
}

/// Shared query behind /notifications and /show-replies.jsp's moderator
/// view + RSS/Atom feed - matches UserEventDao.getRepliesForUser: when
/// `show_private` is false (viewing someone else's feed), events flagged
/// `private` are excluded.
pub(crate) async fn fetch_events(
    state: &AppState,
    user_id: i32,
    db_type: Option<&str>,
    show_private: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<NotificationEvent>> {
    Ok(sqlx::query_as::<_, NotificationEvent>(
        r#"SELECT e.id, e.event_date, t.title AS subj, t.id AS msgid, e.comment_id AS cid, e.unread, e.type::text AS event_type,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  s.name AS section_name, g.urlname AS group_urlname, ou.nick AS origin_nick,
                  COALESCE(ou.nick,cu.nick,tu.nick) AS author_nick,
                  e.message AS event_message,
                  mw.closed_by IS NOT NULL AS closed_warning,
                  CASE WHEN e.type='DEL'::event_type THEN di.bonus ELSE NULL END AS bonus,
                  ARRAY(SELECT tv.value FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                        WHERE tg.msgid=t.id ORDER BY tv.value LIMIT 3) AS tags,
                  mb.message AS message_text,mb.markup::text AS message_markup,
                  CASE WHEN e.type='REACTION'::event_type AND e.origin_user IS NOT NULL
                       THEN COALESCE(c.reactions,t.reactions)->>(e.origin_user::text)
                       ELSE NULL END AS reaction
           FROM user_events e
           JOIN topics t ON t.id=e.message_id
           LEFT JOIN comments c ON c.id=e.comment_id
           LEFT JOIN users ou ON ou.id=e.origin_user
           LEFT JOIN users cu ON cu.id=c.userid
           JOIN users tu ON tu.id=t.userid
           LEFT JOIN message_warnings mw ON mw.id=e.warning_id
           LEFT JOIN del_info di ON di.msgid=CASE WHEN e.comment_id IS NOT NULL THEN e.comment_id ELSE e.message_id END
           JOIN msgbase mb ON mb.id=COALESCE(e.comment_id,e.message_id)
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE e.userid=$1 AND ($2::text IS NULL OR e.type::text=$2) AND ($3 OR NOT e.private)
           ORDER BY e.id DESC LIMIT $4 OFFSET $5"#,
    )
    .bind(user_id)
    .bind(db_type)
    .bind(show_private)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?)
}

#[derive(Debug, Clone)]
struct StPreparedNotification {
    stEvent: NotificationEvent,
    iLastId: i32,
    iCount: usize,
    vecReactions: Vec<(String, String)>,
    vecAuthors: Vec<String>,
}

impl StPreparedNotification {
    fn stFromEvent(stEvent: NotificationEvent) -> Self {
        let vecReactions = match (&stEvent.reaction, &stEvent.origin_nick) {
            (Some(sReaction), Some(sNick)) if stEvent.event_type == "REACTION" => {
                vec![(sReaction.clone(), sNick.clone())]
            }
            _ => Vec::new(),
        };
        let iLastId = stEvent.id;
        Self {
            vecAuthors: vec![stEvent.author_nick.clone()],
            stEvent,
            iLastId,
            iCount: 1,
            vecReactions,
        }
    }
}

/// UserEventPrepareService.prepareGrouped. Events arrive newest first, while
/// the Scala implementation uses foldRight and therefore builds groups from
/// oldest to newest before sorting the prepared result by display date.
fn vecPrepareNotifications(
    vecEvents: Vec<NotificationEvent>,
    bNewDesign: bool,
) -> Vec<StPreparedNotification> {
    let mut vecPrepared: Vec<StPreparedNotification> = Vec::new();

    for stEvent in vecEvents.into_iter().rev() {
        if stEvent.event_type == "WATCH" {
            if let Some(stExisting) = vecPrepared.iter_mut().find(|stExisting| {
                stExisting.stEvent.event_type == "WATCH"
                    && stExisting.stEvent.msgid == stEvent.msgid
                    && stExisting.stEvent.unread == stEvent.unread
            }) {
                stExisting.iCount += 1;
                stExisting.iLastId = stEvent.id;
                if !stExisting.vecAuthors.contains(&stEvent.author_nick) {
                    stExisting.vecAuthors.push(stEvent.author_nick.clone());
                    stExisting.vecAuthors.sort();
                }
                if !stEvent.unread {
                    stExisting.stEvent.event_date = stEvent.event_date;
                    stExisting.stEvent.cid = stEvent.cid;
                }
                continue;
            }
        } else if bNewDesign && stEvent.event_type == "REACTION" {
            let optSimilar = vecPrepared.iter().position(|stExisting| {
                stExisting.stEvent.event_type == "REACTION"
                    && stExisting.stEvent.msgid == stEvent.msgid
                    && stExisting.stEvent.cid == stEvent.cid
                    && stExisting.stEvent.unread == stEvent.unread
                    && (stEvent.event_date - stExisting.stEvent.event_date)
                        .num_seconds()
                        .abs()
                        < 30 * 60
            });
            let optLastSimilar = vecPrepared.len().checked_sub(1).filter(|iIndex| {
                let stExisting = &vecPrepared[*iIndex];
                stExisting.stEvent.event_type == "REACTION"
                    && stExisting.stEvent.msgid == stEvent.msgid
                    && stExisting.stEvent.cid == stEvent.cid
                    && stExisting.stEvent.unread == stEvent.unread
            });
            if let Some(iIndex) = optSimilar.or(optLastSimilar) {
                let stExisting = &mut vecPrepared[iIndex];
                if let (Some(sReaction), Some(sNick)) =
                    (stEvent.reaction.as_ref(), stEvent.origin_nick.as_ref())
                {
                    stExisting
                        .vecReactions
                        .push((sReaction.clone(), sNick.clone()));
                }
                stExisting.iLastId = stEvent.id;
                continue;
            }
        }

        vecPrepared.push(StPreparedNotification::stFromEvent(stEvent));
    }

    vecPrepared.sort_by_key(|stPrepared| std::cmp::Reverse(stPrepared.stEvent.event_date));
    vecPrepared
}

fn bNotificationIsCurrent(stEvent: &NotificationEvent) -> bool {
    stEvent.event_type != "REACTION" || stEvent.reaction.is_some()
}

fn sNotificationIcon(sEventType: &str) -> &'static str {
    match sEventType {
        "DEL" => {
            "<img src=\"/img/del.png\" alt=\"[X]\" title=\"Сообщение удалено\" width=\"15\" height=\"15\">"
        }
        "REPLY" => "<i class=\"icon-reply icon-reply-color\" title=\"Ответ\"></i>",
        "REF" => "<span title=\"Упоминание\">@️</span>",
        "TAG" => "<i class=\"icon-tag icon-tag-color\" title=\"Избранный тег\"></i>",
        "WARNING" => "<span title=\"Уведомление модератора\">⚠️</span>",
        _ => "",
    }
}

fn sNotificationTags(stEvent: &NotificationEvent) -> String {
    stEvent
        .tags
        .iter()
        .map(|sTag| {
            format!(
                "<span class=\"tag\">{}</span>",
                html_escape::encode_text(sTag)
            )
        })
        .collect()
}

fn sNotificationDetails(stEvent: &NotificationEvent) -> String {
    let sMessage = html_escape::encode_text(stEvent.event_message.as_deref().unwrap_or(""));
    match stEvent.event_type.as_str() {
        "DEL" => format!("{sMessage} ({})", stEvent.bonus.unwrap_or(0)),
        "WARNING" if stEvent.closed_warning => format!("<s>{sMessage}</s>"),
        "WARNING" => sMessage.into_owned(),
        _ => String::new(),
    }
}

fn sNotificationAuthor(sNick: &str) -> String {
    format!(
        "<a href=\"/people/{}/profile\">{}</a>",
        urlencoding::encode(sNick),
        html_escape::encode_text(sNick)
    )
}

fn sUnreadDescription(iUnreadCount: i32) -> String {
    let sNoun = if iUnreadCount == 1 || (iUnreadCount > 20 && iUnreadCount % 10 == 1) {
        "непрочитанное уведомление"
    } else if matches!(iUnreadCount, 2 | 3)
        || (iUnreadCount > 20 && matches!(iUnreadCount % 10, 2 | 3))
    {
        "непрочитанных уведомления"
    } else {
        "непрочитанных уведомлений"
    };
    format!("У вас {iUnreadCount} {sNoun}")
}

#[derive(Deserialize)]
pub struct NotificationsQuery {
    pub filter: Option<String>,
    pub offset: Option<i64>,
}

/// UserEventController.showNotifications - requires auth, lists user_events
/// for the current user (newest first), with an "answers/favorites/deleted/
/// reference/tag/reaction/warning" filter and offset pagination.
pub async fn notifications(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<NotificationsQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<axum::response::Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    // Reflected-XSS guard: normalize to a known filter name instead of
    // echoing the raw `?filter=` value back into the page (it's spliced
    // into an href below).
    let requested_filter = q.filter.unwrap_or_else(|| "all".to_string());
    let filter = if requested_filter == "all" || filter_db_type(&requested_filter).is_some() {
        requested_filter
    } else {
        "all".to_string()
    };
    let db_type = filter_db_type(&filter);
    let offset = q.offset.unwrap_or(0).max(0);
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let stSettings = crate::profile::ProfileSettings::from_hstore_text(settings_text);
    let iPageSize = i64::from(stSettings.topics.max(1));

    // Java deliberately loads the retained event window first, removes stale
    // reaction rows, groups it, and only then applies the display offset.
    let vecEvents = fetch_events(&state, user.id, db_type, true, 4000, 0).await?;
    let vecEvents: Vec<_> = vecEvents
        .into_iter()
        .filter(bNotificationIsCurrent)
        .collect();
    let vecPrepared = vecPrepareNotifications(vecEvents, !stSettings.old_notifications);
    let optTopId = vecPrepared.iter().map(|stEvent| stEvent.iLastId).max();
    let vecPage: Vec<_> = vecPrepared
        .into_iter()
        .skip(offset as usize)
        .take(iPageSize as usize)
        .collect();
    // This intentionally mirrors the original JSP model, which treats a
    // completely full page as having a possible next page.
    let bHasMore = vecPage.len() as i64 == iPageSize;
    let iUnreadCount: i32 = sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;

    let vecAvailableTypes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT type::text FROM user_events WHERE userid=$1 ORDER BY type::text",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut html = String::from("<h1>Уведомления</h1><nav>");
    for (label, value, db_value) in [
        ("все", "all", ""),
        ("ответы", "answers", "REPLY"),
        ("отслеживаемое", "favorites", "WATCH"),
        ("удаленное", "deleted", "DEL"),
        ("упоминания", "reference", "REF"),
        ("теги", "tag", "TAG"),
        ("реакции", "reaction", "REACTION"),
        ("предупреждения", "warning", "WARNING"),
    ] {
        if !db_value.is_empty() && !vecAvailableTypes.iter().any(|sType| sType == db_value) {
            continue;
        }
        let active = if value == filter {
            "btn-selected"
        } else {
            "btn-default"
        };
        html.push_str(&format!(
            "<a class=\"btn {active}\" href=\"/notifications?filter={value}\">{label}</a> "
        ));
    }
    html.push_str("</nav>");
    if iUnreadCount > 0 {
        html.push_str(&format!(
            "<div id=\"counter_block\" class=\"infoblock\" data-unread-count=\"{iUnreadCount}\"><span id=\"counter_text\">{}</span>",
            sUnreadDescription(iUnreadCount)
        ));
    }
    if iUnreadCount > 0
        && let Some(top_id) = optTopId
    {
        html.push_str(&format!(
            "<form id=\"reset_form\" method=\"post\" action=\"/notifications\" style=\"display:inline\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf_token}\"><input type=\"hidden\" name=\"topId\" value=\"{top_id}\"><button type=\"submit\">Сбросить все</button></form>",
            csrf_token = html_escape::encode_double_quoted_attribute(&csrf_token),
        ));
        if stSettings.old_notifications {
            // show-replies.jsp resets unread state immediately after the old
            // page loads; keep the same browser-side side effect without
            // requiring the original jQuery helper.
            html.push_str(
                "<script>document.addEventListener('DOMContentLoaded',function(){var f=document.getElementById('reset_form');if(!f)return;fetch('/notifications-reset',{method:'POST',body:new URLSearchParams(new FormData(f))}).then(function(r){if(r.ok)f.style.display='none';});});</script>",
            );
        }
    }
    if iUnreadCount > 0 {
        html.push_str("</div>");
    }

    if stSettings.old_notifications {
        html.push_str("<div class=\"forum\"><table class=\"message-table\" width=\"100%\">");
        for stPrepared in &vecPage {
            let stEvent = &stPrepared.stEvent;
            let sSubjectPlain = stEvent.sSubjectPlain();
            let sReaction = stPrepared
                .vecReactions
                .first()
                .map(|(sReaction, _)| sReaction.as_str())
                .unwrap_or_default();
            let sIcon = if stEvent.event_type == "REACTION" {
                html_escape::encode_text(sReaction).into_owned()
            } else {
                sNotificationIcon(&stEvent.event_type).to_owned()
            };
            let sTags = sNotificationTags(stEvent);
            let sDetails = sNotificationDetails(stEvent);
            let sDetails = if !sDetails.is_empty() {
                format!("<br>{sDetails}")
            } else {
                String::new()
            };
            let sUnreadMark = if stEvent.unread { "•" } else { "" };
            let sAuthorOrCount = if stPrepared.iCount > 1 {
                format!("<i class=\"icon-comment\"></i> {}", stPrepared.iCount)
            } else {
                sNotificationAuthor(&stEvent.author_nick)
            };
            let sDate = crate::request_timezone::sTimeTag("interval", stEvent.event_date);
            html.push_str(&format!(
                "<tr><td align=\"center\">{icon}</td><td><a href=\"{link}\" class=\"event-unread-{unread}\">{tags}{subj}</a> ({section}){details} {unread_mark}</td><td title=\"{authors}\">{date}, {author_or_count}</td></tr>",
                icon = sIcon,
                link = html_escape::encode_double_quoted_attribute(&stEvent.link()),
                unread = stEvent.unread,
                tags = sTags,
                subj = html_escape::encode_text(&sSubjectPlain),
                section = html_escape::encode_text(&stEvent.section_name),
                details = sDetails,
                unread_mark = sUnreadMark,
                authors = html_escape::encode_double_quoted_attribute(&stPrepared.vecAuthors.join(", ")),
                date = sDate,
                author_or_count = sAuthorOrCount,
            ));
        }
        html.push_str("</table></div>");
    } else {
        html.push_str("<div class=\"notifications\">");
        for stPrepared in &vecPage {
            let stEvent = &stPrepared.stEvent;
            let sSubjectPlain = stEvent.sSubjectPlain();
            let sDate = crate::request_timezone::sTimeTag("compact-interval", stEvent.event_date);
            let sUnreadClass = if stEvent.unread {
                "event-unread-true"
            } else {
                "event-unread-false"
            };
            let iUnreadDelta = stPrepared.iCount.max(stPrepared.vecReactions.len());
            html.push_str(&format!(
                "<form action=\"/notifications-click\" method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input type=\"hidden\" name=\"firstId\" value=\"{first}\"><input type=\"hidden\" name=\"lastId\" value=\"{last}\"><button type=\"submit\" class=\"{unread} notifications-item\" data-unread-delta=\"{delta}\"><div class=\"notifications-type\"><p>{icon}</p></div><div class=\"notifications-title\"><p>{comment_icon}{subj} ({section})</p></div>",
                csrf = html_escape::encode_double_quoted_attribute(&csrf_token),
                first = stEvent.id,
                last = stPrepared.iLastId,
                unread = sUnreadClass,
                delta = iUnreadDelta,
                icon = sNotificationIcon(&stEvent.event_type),
                comment_icon = stEvent.cid.map(|_| "<i class=\"icon-comment\"></i>").unwrap_or(""),
                subj = html_escape::encode_text(&sSubjectPlain),
                section = html_escape::encode_text(&stEvent.section_name),
            ));
            if !stPrepared.vecReactions.is_empty() {
                html.push_str(
                    "<div class=\"notifications-reactions\"><p><span class=\"reactions\">",
                );
                for (sReaction, sNick) in &stPrepared.vecReactions {
                    html.push_str(&format!(
                        "<span class=\"reaction\">{} {}</span>",
                        html_escape::encode_text(sReaction),
                        html_escape::encode_text(sNick),
                    ));
                }
                html.push_str("</span></p></div>");
            } else if stPrepared.iCount > 1 {
                html.push_str(&format!(
                    "<div title=\"{}\" class=\"notifications-number\"><p><i class=\"icon-comment\"></i> {}</p></div>",
                    html_escape::encode_double_quoted_attribute(&stPrepared.vecAuthors.join(", ")),
                    stPrepared.iCount,
                ));
            } else {
                let sDetails = if stEvent.event_type == "TAG" {
                    sNotificationTags(stEvent)
                } else {
                    sNotificationDetails(stEvent)
                };
                html.push_str(&format!(
                    "<div class=\"notifications-details\"><p>{sDetails}</p></div><div class=\"notifications-who-when\"><p>{}, {}</p></div>",
                    sNotificationAuthor(&stEvent.author_nick),
                    sDate,
                ));
            }
            if !stPrepared.vecReactions.is_empty() || stPrepared.iCount > 1 {
                html.push_str(&format!(
                    "<div class=\"notifications-when\"><p>{}</p></div>",
                    sDate
                ));
            }
            html.push_str("</button></form>");
        }
        html.push_str("</div>");
    }
    if vecPage.is_empty() {
        html.push_str("<p class=\"muted\">Нет уведомлений</p>");
    }

    if offset > 0 {
        html.push_str(&format!(
            "<a href=\"/notifications?filter={filter}&offset={}\">← предыдущие</a> ",
            offset - iPageSize
        ));
    }
    if bHasMore {
        html.push_str(&format!(
            "<a href=\"/notifications?filter={filter}&offset={}\">Далее »</a>",
            offset + iPageSize
        ));
    }

    html.push_str(&format!(
        "<p><i class=\"icon-rss\"></i> <a href=\"/show-replies.jsp?output=rss&amp;nick={}\">RSS подписка на новые уведомления</a></p>",
        urlencoding::encode(&user.nick)
    ));

    let sPage = StNotificationsTemplate {
        sContentHtml: html,
        sNick: user.nick,
    }
    .render()?;
    let mut stResponse = Html(sPage).into_response();
    stResponse.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(stResponse)
}

#[derive(Deserialize)]
pub struct NotificationsResetForm {
    #[serde(rename = "topId")]
    pub top_id: i32,
}

async fn reset_unread_events(state: &AppState, user_id: i32, top_id: i32) -> Result<()> {
    sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND id<=$2")
        .bind(user_id)
        .bind(top_id)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// POST /notifications (UserEventController.resetNotifications) - HTML flow,
/// redirects back to the notifications page.
pub async fn notifications_mark_read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Form(form): axum::Form<NotificationsResetForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    reset_unread_events(&state, user.id, form.top_id).await?;
    state.realtime.vNotifyEvents([user.id]);
    Ok(Redirect::to("/notifications"))
}

/// GET /notifications-count (UserEventApiController.getEventsCount) - bare
/// JSON integer, not an object, matching the Java `Json` response shape.
pub async fn notifications_count(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let count: i32 = sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!(count)))
}

/// POST /notifications-reset (UserEventApiController.resetNotifications) -
/// the JSON-API twin of the HTML `notifications_mark_read` above.
pub async fn notifications_reset(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Form(form): axum::Form<NotificationsResetForm>,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    reset_unread_events(&state, user.id, form.top_id).await?;
    state.realtime.vNotifyEvents([user.id]);
    Ok(Json(json!("ok")))
}

#[derive(Debug, serde::Deserialize)]
pub struct TrackerQuery {
    pub offset: Option<i64>,
    pub filter: Option<String>,
}

#[derive(Template)]
#[template(path = "tracker.html")]
struct TrackerTemplate {
    title: String,
    filter: String,
    default_filter: String,
    topics: Vec<TrackerTopic>,
    prev_link: Option<String>,
    next_link: Option<String>,
    is_moderator: bool,
    old_tracker: bool,
    uncommitted: Vec<(i32, String, i64)>,
    new_users: Vec<TrackerModeratorUser>,
    frozen_users: Vec<TrackerModeratorUser>,
    unfrozen_users: Vec<TrackerModeratorUser>,
    blocked_users: Vec<TrackerModeratorUser>,
    unblocked_users: Vec<TrackerModeratorUser>,
    recent_userpics: Vec<TrackerModeratorUserpic>,
    blocked_ips: Vec<String>,
    unblocked_ips: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct TrackerModeratorUser {
    nick: String,
    bold: bool,
    strike: bool,
}

impl TrackerModeratorUser {
    fn profile_url(&self) -> String {
        format!("/people/{}/profile", urlencoding::encode(&self.nick))
    }
}

#[derive(Debug)]
struct TrackerModeratorUserpic {
    profile_url: String,
    image_url: String,
    width: i32,
    height: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct TrackerTopicRow {
    id: i32,
    title: String,
    postdate: chrono::DateTime<chrono::Utc>,
    topic_author: String,
    author: String,
    group_title: String,
    group_urlname: String,
    section_prefix: String,
    comments: i32,
    raw_comments: i32,
    resolved: bool,
    tags: Option<String>,
    last_comment_id: Option<i32>,
    comments_closed: bool,
    uncommitted: bool,
}

#[derive(Debug)]
struct TrackerTopic {
    stRow: TrackerTopicRow,
    iPages: i32,
}

impl TrackerTopic {
    fn sTitlePlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.stRow.title)
    }

    fn sGroupUrl(&self) -> String {
        format!(
            "/{}/{}/",
            self.stRow.section_prefix, self.stRow.group_urlname
        )
    }

    fn sLastPageUrl(&self) -> String {
        let iLastCommentId = self.stRow.last_comment_id.unwrap_or(0);
        if self.iPages > 1 {
            format!(
                "{}{}/page{}?lastmod={iLastCommentId}",
                self.sGroupUrl(),
                self.stRow.id,
                self.iPages - 1
            )
        } else {
            format!(
                "{}{}?lastmod={iLastCommentId}",
                self.sGroupUrl(),
                self.stRow.id
            )
        }
    }

    fn vecTags(&self) -> Vec<&str> {
        self.stRow
            .tags
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|sValue| !sValue.is_empty())
            .collect()
    }
}

fn sTrackerOldLocation(optFilter: Option<&str>, sDefaultFilter: &str) -> String {
    // @RequestParam(defaultValue = "all") followed by
    // TrackerFilterEnum.getByValue: invalid and empty values are deliberately
    // preserved, while the user's own default is omitted from the canonical
    // tracker URL.
    let sFilter = optFilter.unwrap_or("all");
    if ["all", "main", "notalks", "tech"].contains(&sFilter) && sFilter == sDefaultFilter {
        "/tracker/".to_owned()
    } else {
        format!("/tracker/?filter={}", urlencoding::encode(sFilter))
    }
}

async fn stTrackerProfile(
    stState: &AppState,
    optUser: Option<&crate::models::UserSummary>,
) -> Result<crate::profile::ProfileSettings> {
    let optSettings = if let Some(stUser) = optUser {
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(stUser.id)
            .fetch_optional(&stState.pool)
            .await?
    } else {
        None
    };
    Ok(crate::profile::ProfileSettings::from_hstore_text(
        optSettings,
    ))
}

pub async fn tracker_old_redirect(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<TrackerQuery>,
) -> Result<axum::response::Response> {
    let stProfile = stTrackerProfile(&stState, optUser.as_ref()).await?;
    let sLocation = sTrackerOldLocation(stQuery.filter.as_deref(), &stProfile.tracker_mode);
    Ok((
        axum::http::StatusCode::FOUND,
        [(axum::http::header::LOCATION, sLocation)],
    )
        .into_response())
}

/// Matches TrackerFilterEnum.NonTech (SectionController.NonTech): these are
/// real production group ids on the upstream Java site (a "Talks" group and
/// a few others) - hardcoded the same way upstream hardcodes them, for
/// compatibility with a migrated real DB. Harmless no-op against fresh dev
/// seed data, which doesn't use these ids.
const TRACKER_NON_TECH_GROUPS: &[i32] = &[8404, 4068, 9326, 19405];
const TRACKER_TALKS_GROUP: i32 = 8404;
/// Forum section id (matches Section.Forum / seed data: 'Форум').
const TRACKER_TECH_SECTION_ID: i32 = 2;
/// `GroupListDao.NoUncommited`: committed topics in premoderated sections
/// plus every topic in postmoderated sections.
const TRACKER_PUBLIC_TOPICS_CLAUSE: &str = "AND (t.moderate OR NOT s.moderate)";

fn tracker_commit_visibility_clause(show_uncommitted: bool) -> &'static str {
    if show_uncommitted {
        ""
    } else {
        TRACKER_PUBLIC_TOPICS_CLAUSE
    }
}

/// `TopicDao.getUncommitedCounts`, including its three-month window.
const UNCOMMITTED_COUNTS_SQL: &str = r#"SELECT s.id,s.name,count(t.id)
    FROM sections s
    JOIN groups g ON g.section=s.id
    JOIN topics t ON t.groupid=g.id
    WHERE s.moderate AND NOT t.moderate
      AND NOT t.deleted AND NOT t.draft
      AND t.postdate > (CURRENT_TIMESTAMP-'3 month'::interval)
    GROUP BY s.id,s.name
    HAVING count(t.id)>0
    ORDER BY s.id"#;

async fn stTrackerModeratorData(
    stState: &AppState,
) -> Result<(
    Vec<TrackerModeratorUser>,
    Vec<TrackerModeratorUser>,
    Vec<TrackerModeratorUser>,
    Vec<TrackerModeratorUser>,
    Vec<TrackerModeratorUser>,
    Vec<TrackerModeratorUserpic>,
    Vec<String>,
    Vec<String>,
)> {
    let vecNewUsers = sqlx::query_as::<_, TrackerModeratorUser>(
        r#"SELECT nick,activated AS bold,blocked AS strike FROM users
           WHERE regdate IS NOT NULL
             AND regdate>CURRENT_TIMESTAMP-'3 days'::interval
           ORDER BY regdate"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let vecFrozenUsers = sqlx::query_as::<_, TrackerModeratorUser>(
        r#"SELECT nick,COALESCE(lastlogin>CURRENT_TIMESTAMP-'1 day'::interval,false) AS bold,
                  false AS strike
           FROM users
           WHERE frozen_until>CURRENT_TIMESTAMP AND NOT blocked
           ORDER BY frozen_until"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let vecUnfrozenUsers = sqlx::query_as::<_, TrackerModeratorUser>(
        r#"SELECT nick,COALESCE(lastlogin>CURRENT_TIMESTAMP-'1 day'::interval,false) AS bold,
                  false AS strike
           FROM users
           WHERE frozen_until<CURRENT_TIMESTAMP
             AND frozen_until>CURRENT_TIMESTAMP-'3 days'::interval
             AND NOT blocked
           ORDER BY frozen_until"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let vecBlockedUsers = sqlx::query_as::<_, TrackerModeratorUser>(
        r#"SELECT u.nick,false AS bold,u.blocked AS strike
           FROM user_log l JOIN users u ON u.id=l.userid
           WHERE l.action='block_user'::user_log_action
             AND l.action_date>CURRENT_TIMESTAMP-'3 days'::interval
           ORDER BY l.action_date"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let vecUnblockedUsers = sqlx::query_as::<_, TrackerModeratorUser>(
        r#"SELECT u.nick,false AS bold,u.blocked AS strike
           FROM user_log l JOIN users u ON u.id=l.userid
           WHERE l.action='unblock_user'::user_log_action
             AND l.action_date>CURRENT_TIMESTAMP-'3 days'::interval
           ORDER BY l.action_date"#,
    )
    .fetch_all(&stState.pool)
    .await?;

    // UserService.getRecentUserpics uses the first occurrence of each user
    // from the three-day, action-date-ordered set and drops DisabledUserpic.
    let vecRecentPhotoRows: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        r#"SELECT u.id,u.nick,u.photo
           FROM user_log l JOIN users u ON u.id=l.userid
           WHERE l.action='set_userpic'::user_log_action
             AND l.action_date>CURRENT_TIMESTAMP-'3 days'::interval
           ORDER BY l.action_date"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let mut setRecentPhotoUsers = std::collections::HashSet::new();
    let vecRecentUserpics = vecRecentPhotoRows
        .into_iter()
        .filter(|(iUserId, _, _)| setRecentPhotoUsers.insert(*iUserId))
        .filter_map(|(_, sNick, optPhoto)| {
            let stUserpic = crate::profile::stResolveUserpic(
                std::path::Path::new(&stState.config.upload_dir),
                "empty",
                false,
                false,
                optPhoto.as_deref(),
                None,
            );
            (stUserpic.sUrl != crate::profile::DISABLED_USERPIC).then(|| TrackerModeratorUserpic {
                profile_url: format!("/people/{}/profile", urlencoding::encode(&sNick)),
                image_url: stUserpic.sUrl,
                width: stUserpic.iWidth,
                height: stUserpic.iHeight,
            })
        })
        .collect();
    let vecBlockedIps: Vec<String> = sqlx::query_scalar(
        r#"SELECT ip::text FROM b_ips
           WHERE date>CURRENT_TIMESTAMP-'3 days'::interval
             AND ban_date>CURRENT_TIMESTAMP AND mod_id<>0
           ORDER BY date"#,
    )
    .fetch_all(&stState.pool)
    .await?;
    let vecUnblockedIps: Vec<String> = sqlx::query_scalar(
        r#"SELECT ip::text FROM b_ips
           WHERE ban_date<CURRENT_TIMESTAMP
             AND ban_date>CURRENT_TIMESTAMP-'3 days'::interval AND mod_id<>0
           ORDER BY ban_date"#,
    )
    .fetch_all(&stState.pool)
    .await?;

    Ok((
        vecNewUsers,
        vecFrozenUsers,
        vecUnfrozenUsers,
        vecBlockedUsers,
        vecUnblockedUsers,
        vecRecentUserpics,
        vecBlockedIps,
        vecUnblockedIps,
    ))
}

fn tracker_filter_group_clause(filter: &str) -> String {
    let non_tech = TRACKER_NON_TECH_GROUPS
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    match filter {
        "notalks" => format!("AND t.groupid <> {TRACKER_TALKS_GROUP} AND NOT t.notop"),
        "main" => format!("AND t.groupid NOT IN ({non_tech}) AND NOT t.notop"),
        "tech" => format!(
            "AND t.groupid NOT IN ({non_tech}) AND NOT t.notop AND s.id = {TRACKER_TECH_SECTION_ID}"
        ),
        _ => String::new(),
    }
}

/// GroupListDao.getTrackerTopics, including topic-author, ignored-tag and
/// branch-author filtering for the last visible comment.
pub async fn tracker(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<TrackerQuery>,
) -> Result<Html<String>> {
    if q.offset.unwrap_or(0) < 0 || q.offset.unwrap_or(0) > 300 {
        return Err(AppError::BadRequest("Некорректное значение offset".into()));
    }
    let offset = q.offset.unwrap_or(0).clamp(0, 300);
    // GroupListDao.getTrackerTopics uses session.profile.topics, not a
    // global page size - each user's own "topics per page" setting.
    let stProfile = stTrackerProfile(&state, user.as_ref()).await?;
    let limit = i64::from(stProfile.topics);
    let default_filter = stProfile.tracker_mode.clone();
    let filter = q
        .filter
        .filter(|f| ["all", "main", "notalks", "tech"].contains(&f.as_str()))
        .unwrap_or_else(|| default_filter.clone());

    // GroupListDao.getTrackerTopics: showUncommited = filter==ALL ||
    // session.moderator || session.corrector.
    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    let is_corrector = user.as_ref().map(|u| u.corrector).unwrap_or(false);
    let show_uncommitted = filter == "all" || is_moderator || is_corrector;

    let sql = format!(
        r#"SELECT t.id, t.title,
                  GREATEST(t.postdate,COALESCE(lc.postdate,t.postdate)) AS postdate,
                  u.nick AS topic_author, COALESCE(lu.nick,u.nick) AS author,
                  g.title AS group_title, g.urlname AS group_urlname,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  CASE WHEN t.postscore IS DISTINCT FROM 10002 THEN t.stat1 ELSE 0 END AS comments,
                  t.stat1 AS raw_comments,
                  COALESCE(t.resolved,false) AS resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags,
                  lc.id AS last_comment_id,
                  COALESCE(t.postscore,-9999) >= 10000 AS comments_closed,
                  s.moderate AND NOT t.moderate AS uncommitted
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN LATERAL (
             SELECT c.id,c.userid,c.postdate
             FROM comments c
             WHERE c.topic=t.id AND NOT c.deleted
               AND ($3::int IS NULL OR NOT EXISTS (
                 SELECT ignored FROM ignore_list WHERE userid=$3
                 INTERSECT SELECT get_branch_authors(c.id)
               ))
             ORDER BY c.postdate DESC
             LIMIT 1
           ) lc ON t.postscore IS DISTINCT FROM 10002
           LEFT JOIN users lu ON lu.id=lc.userid
           WHERE NOT t.draft AND NOT t.deleted
             AND COALESCE(t.lastmod, t.postdate) > now() - interval '7 days'
             AND ($3::int IS NULL OR t.userid NOT IN (
               SELECT ignored FROM ignore_list WHERE userid=$3
             ))
             AND ($3::int IS NULL OR NOT (
               EXISTS (
                 SELECT 1 FROM tags tg
                 JOIN user_tags ignored_tag ON ignored_tag.tag_id=tg.tagid
                 WHERE tg.msgid=t.id AND ignored_tag.user_id=$3
                   AND NOT ignored_tag.is_favorite
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tags tg
                 JOIN user_tags favorite_tag ON favorite_tag.tag_id=tg.tagid
                 WHERE tg.msgid=t.id AND favorite_tag.user_id=$3
                   AND favorite_tag.is_favorite
               )
             ))
             AND GREATEST(t.postdate,COALESCE(lc.postdate,t.postdate)) > now() - interval '7 days'
             {uncommitted}
             {open_warnings}
             {group_clause}
           ORDER BY GREATEST(t.postdate,COALESCE(lc.postdate,t.postdate)) DESC
           OFFSET $1 LIMIT $2"#,
        uncommitted = tracker_commit_visibility_clause(show_uncommitted),
        // TopicPermissionService's noHidden clause: only applied to
        // unauthorized (anonymous) viewers - an authorized user of any
        // score can see warned topics in the tracker.
        open_warnings = if user.is_none() {
            "AND t.open_warnings <= 2"
        } else {
            ""
        },
        group_clause = tracker_filter_group_clause(&filter),
    );
    let vecRows = sqlx::query_as::<_, TrackerTopicRow>(sqlx::AssertSqlSafe(sql))
        .bind(offset)
        .bind(limit)
        .bind(user.as_ref().map(|stUser| stUser.id))
        .fetch_all(&state.pool)
        .await?;
    let iMessages = stProfile.messages.max(1);
    let topics = vecRows
        .into_iter()
        .map(|stRow| TrackerTopic {
            iPages: ((stRow.raw_comments.max(0) + iMessages - 1) / iMessages).max(0),
            stRow,
        })
        .collect::<Vec<_>>();

    let filter_label = match filter.as_str() {
        "main" => "основные",
        "notalks" => "без talks",
        "tech" => "тех. форум",
        _ => "все",
    };
    let title = if filter == default_filter {
        "Активные топики".to_string()
    } else {
        format!("Активные топики ({filter_label})")
    };
    let extra = if filter == default_filter {
        String::new()
    } else {
        format!("filter={}", urlencoding::encode(&filter))
    };
    let next_link = if topics.len() as i64 == limit && offset < 300 {
        let sep = if extra.is_empty() { "" } else { "&" };
        Some(format!(
            "/tracker/?offset={}{}{}",
            offset + limit,
            sep,
            extra
        ))
    } else {
        None
    };
    let prev_link = if offset >= limit {
        let new_offset = offset - limit;
        if extra.is_empty() {
            Some(if new_offset == 0 {
                "/tracker/".to_string()
            } else {
                format!("/tracker/?offset={new_offset}")
            })
        } else {
            Some(if new_offset == 0 {
                format!("/tracker/?{extra}")
            } else {
                format!("/tracker/?offset={new_offset}&{extra}")
            })
        }
    } else {
        None
    };
    let uncommitted = if is_moderator || is_corrector {
        sqlx::query_as::<_, (i32, String, i64)>(UNCOMMITTED_COUNTS_SQL)
            .fetch_all(&state.pool)
            .await?
    } else {
        Vec::new()
    };
    let (
        new_users,
        frozen_users,
        unfrozen_users,
        blocked_users,
        unblocked_users,
        recent_userpics,
        blocked_ips,
        unblocked_ips,
    ) = if is_moderator {
        stTrackerModeratorData(&state).await?
    } else {
        Default::default()
    };
    Ok(Html(
        TrackerTemplate {
            title,
            filter,
            default_filter,
            topics,
            prev_link,
            next_link,
            is_moderator,
            old_tracker: stProfile.old_tracker,
            uncommitted,
            new_users,
            frozen_users,
            unfrozen_users,
            blocked_users,
            unblocked_users,
            recent_userpics,
            blocked_ips,
            unblocked_ips,
        }
        .render()?,
    ))
}

#[derive(Template)]
#[template(path = "topiclist_boxlet.html")]
struct StTopicListBoxletTemplate {
    sName: &'static str,
    sLink: Option<&'static str>,
    sDescription: &'static str,
    vecItems: Vec<StTopicBoxletItem>,
}

#[derive(Template)]
#[template(path = "poll_boxlet.html")]
struct StPollBoxletTemplate {
    stPoll: StPollBoxlet,
    bAuthorized: bool,
    bEnabled: bool,
    sCsrfToken: String,
    sResultsUrl: String,
}

/// `TopTenBoxlet`: public and method-agnostic through `AbstractBoxlet`, but
/// pagination links use the authenticated visitor's `profile.messages`.
pub async fn top10_boxlet(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
) -> Result<axum::response::Response> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    let iMessagesPerPage = cService
        .iMessagesPerPage(optUser.as_ref().map(|stUser| stUser.id))
        .await?;
    Ok(crate::routes::boxlets::stHtmlFragment(
        sRenderTop10Boxlet(&stState, iMessagesPerPage).await?,
    ))
}

pub(crate) async fn sRenderTop10Boxlet(
    stState: &AppState,
    iMessagesPerPage: i32,
) -> Result<String> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    Ok(StTopicListBoxletTemplate {
        sName: "Top 10",
        sLink: None,
        sDescription: "Наиболее обсуждаемые темы этого месяца",
        vecItems: cService.vecTop10(iMessagesPerPage).await?,
    }
    .render()?)
}

/// `ArticlesBoxlet`: the ten newest committed, visible article topics.
pub async fn articles_boxlet(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
) -> Result<axum::response::Response> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    let iMessagesPerPage = cService
        .iMessagesPerPage(optUser.as_ref().map(|stUser| stUser.id))
        .await?;
    Ok(crate::routes::boxlets::stHtmlFragment(
        sRenderArticlesBoxlet(&stState, iMessagesPerPage).await?,
    ))
}

pub(crate) async fn sRenderArticlesBoxlet(
    stState: &AppState,
    iMessagesPerPage: i32,
) -> Result<String> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    Ok(StTopicListBoxletTemplate {
        sName: "Статьи",
        sLink: Some("/articles/"),
        sDescription: "Новые статьи",
        vecItems: cService.vecArticles(iMessagesPerPage).await?,
    }
    .render()?)
}

/// `PollBoxlet`: anonymous visitors see disabled controls and the login
/// prompt, eligible users get the voting form, and voters see their selected
/// variants plus the results link without `results=true`.
pub async fn poll_boxlet(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<axum::response::Response> {
    let bAuthorized = optUser.is_some();
    Ok(crate::routes::boxlets::stHtmlFragment(
        sRenderPollBoxlet(
            &stState,
            optUser.as_ref().map(|stUser| stUser.id),
            bAuthorized,
            sCsrfToken,
        )
        .await?,
    ))
}

pub(crate) async fn sRenderPollBoxlet(
    stState: &AppState,
    optUserId: Option<i32>,
    bAuthorized: bool,
    sCsrfToken: String,
) -> Result<String> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    let stPoll = cService.stPoll(optUserId).await?;
    let bEnabled = bAuthorized && !stPoll.bUserVoted;
    let sResultsUrl = format!(
        "/polls/polls/{}{}",
        stPoll.iTopicId,
        if stPoll.bUserVoted && bAuthorized {
            ""
        } else {
            "?results=true"
        }
    );
    Ok(StPollBoxletTemplate {
        stPoll,
        bAuthorized,
        bEnabled,
        sCsrfToken,
        sResultsUrl,
    }
    .render()?)
}

#[cfg(test)]
mod boxlet_endpoint_tests {
    use askama::Template;

    use super::{StPollBoxletTemplate, StTopicListBoxletTemplate};
    use crate::domain::boxlet::model::{StPollBoxlet, StPollVariantResult, StTopicBoxletItem};

    #[test]
    fn topic_list_template_matches_original_heading_page_and_count_dom() {
        let sHtml = StTopicListBoxletTemplate {
            sName: "Статьи",
            sLink: Some("/articles/"),
            sDescription: "Новые статьи",
            vecItems: vec![StTopicBoxletItem {
                sMessageUrl: "/articles/test/42".to_owned(),
                sTitle: "<script>unsafe</script>".to_owned(),
                iCommentCount: 51,
                iPages: 3,
                optLastPageUrl: Some("/articles/test/42/page2?lastmod=1725000000123".to_owned()),
            }],
        }
        .render()
        .expect("topic list template");

        assert!(sHtml.contains("<h2><a href=\"/articles/\">Статьи</a></h2>"));
        assert!(sHtml.contains("Новые статьи:"));
        assert!(sHtml.contains(
            "(стр.&nbsp;<a href=\"/articles/test/42/page2?lastmod=1725000000123\">3</a>)"
        ));
        assert!(sHtml.contains("(51)"));
        assert!(!sHtml.contains("<script>"));
    }

    #[test]
    fn topic_and_poll_titles_are_escaped_once_after_database_entity_decode() {
        let sTopicHtml = StTopicListBoxletTemplate {
            sName: "Top 10",
            sLink: None,
            sDescription: "Темы",
            vecItems: vec![StTopicBoxletItem {
                sMessageUrl: "/forum/test/42".to_owned(),
                sTitle: "A & B < C \"Q\" 'X' A 😀".to_owned(),
                iCommentCount: 1,
                iPages: 1,
                optLastPageUrl: None,
            }],
        }
        .render()
        .expect("topic list template");
        assert!(
            sTopicHtml.contains("A &#38; B &#60; C &#34;Q&#34; &#39;X&#39; A 😀"),
            "rendered topic boxlet: {sTopicHtml}"
        );
        assert!(!sTopicHtml.contains("&#38;amp;"));
        assert!(!sTopicHtml.contains("&#38;lt;"));
        assert!(!sTopicHtml.contains("&#38;quot;"));
        assert!(!sTopicHtml.contains("&#38;#39;"));
        assert!(!sTopicHtml.contains("&#38;#x41;"));

        let sPollHtml = sRenderPoll(
            StPollBoxlet {
                sTitle: "A & B < C «Q» 'X' A 😀".to_owned(),
                ..stPoll(false)
            },
            false,
            false,
            "/polls/polls/77?results=true",
        );
        assert!(sPollHtml.contains("A &#38; B &#60; C «Q» &#39;X&#39; A 😀"));
        assert!(!sPollHtml.contains("&#38;amp;"));
        assert!(!sPollHtml.contains("&#38;lt;"));
        assert!(!sPollHtml.contains("&#38;quot;"));
        assert!(!sPollHtml.contains("&#38;#39;"));
        assert!(!sPollHtml.contains("&#38;#x41;"));
    }

    #[test]
    fn anonymous_poll_has_disabled_inputs_login_prompt_and_forced_results() {
        let sHtml = sRenderPoll(stPoll(false), false, false, "/polls/polls/77?results=true");

        assert!(!sHtml.contains("<form action=\"/vote.jsp\""));
        assert!(sHtml.contains("<input type=\"radio\" disabled name=\"vote\" value=\"1\">"));
        assert!(sHtml.contains("/login.jsp?from=%2Fpoll.boxlet"));
        assert!(sHtml.contains("href=\"register.jsp\""));
        assert!(sHtml.contains("href=\"/polls/polls/77?results=true\">результаты</a>"));
    }

    #[test]
    fn authorized_non_voter_gets_csrf_form_and_enabled_controls() {
        let sHtml = sRenderPoll(stPoll(false), true, true, "/polls/polls/77?results=true");

        assert!(sHtml.contains("<form action=\"/vote.jsp\" method=\"POST\""));
        assert!(sHtml.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(sHtml.contains("name=\"voteid\" value=\"7\""));
        assert!(sHtml.contains("<input type=\"radio\" name=\"vote\" value=\"1\">"));
        assert!(!sHtml.contains("<input type=\"radio\" disabled"));
        assert!(!sHtml.contains("Для участия в опросе"));
    }

    #[test]
    fn voter_sees_penguin_selected_variant_and_plain_results_link() {
        let sHtml = sRenderPoll(stPoll(true), true, false, "/polls/polls/77");

        assert!(sHtml.contains("class=\"penguin_progress\""));
        assert!(!sHtml.contains("<form action=\"/vote.jsp\""));
        assert!(!sHtml.contains("Для участия в опросе"));
        assert!(sHtml.contains("href=\"/polls/polls/77\">результаты</a>"));
        assert!(!sHtml.contains("results=true"));
    }

    fn stPoll(bUserVoted: bool) -> StPollBoxlet {
        StPollBoxlet {
            iPollId: 7,
            iTopicId: 77,
            bMultiSelect: false,
            sTitle: "Лучший язык?".to_owned(),
            vecVariants: vec![
                StPollVariantResult {
                    iId: 1,
                    sLabel: "Rust".to_owned(),
                    iVotes: 4,
                    bUserVoted,
                },
                StPollVariantResult {
                    iId: 2,
                    sLabel: "C++".to_owned(),
                    iVotes: 2,
                    bUserVoted: false,
                },
            ],
            iVotes: 6,
            iUsers: 5,
            bUserVoted,
        }
    }

    fn sRenderPoll(
        stPoll: StPollBoxlet,
        bAuthorized: bool,
        bEnabled: bool,
        sResultsUrl: &str,
    ) -> String {
        StPollBoxletTemplate {
            stPoll,
            bAuthorized,
            bEnabled,
            sCsrfToken: "csrf-token".to_owned(),
            sResultsUrl: sResultsUrl.to_owned(),
        }
        .render()
        .expect("poll template")
    }
}

#[derive(serde::Deserialize)]
pub struct ReactionQuery {
    pub topic: Option<i32>,
    pub comment: Option<i32>,
    /// Not consumed by the controller, but read by `topic.tag` while it
    /// renders a poll on the reaction page.  JSP compares the raw value to
    /// the lower-case string `true` rather than deserializing a boolean.
    pub results: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ReactionForm {
    pub topic: Option<i32>,
    pub comment: Option<i32>,
    pub msgid: Option<i32>,
    pub reaction: Option<String>,
    pub value: Option<bool>,
}

fn parse_reaction_action(raw: Option<String>, value: Option<bool>) -> (String, bool) {
    let raw = raw.unwrap_or_else(|| "+1-true".to_string());
    if let Some((reaction, action)) = raw.rsplit_once('-')
        && (action == "true" || action == "false")
    {
        return (reaction.to_string(), action == "true");
    }
    (raw, value.unwrap_or(true))
}

async fn resolve_reaction_target(
    pool: &sqlx::PgPool,
    topic: Option<i32>,
    comment: Option<i32>,
    msgid: Option<i32>,
) -> Result<(i32, Option<i32>)> {
    if let Some(comment_id) = comment {
        let topic_id: i32 = sqlx::query_scalar("SELECT topic FROM comments WHERE id=$1")
            .bind(comment_id)
            .fetch_optional(pool)
            .await?
            .ok_or(crate::error::AppError::NotFound)?;
        return Ok((topic_id, Some(comment_id)));
    }

    let topic_id = topic
        .or(msgid)
        .ok_or_else(|| crate::error::AppError::BadRequest("missing topic/comment".into()))?;
    Ok((topic_id, None))
}

async fn reaction_target_link(
    pool: &sqlx::PgPool,
    topic_id: i32,
    comment_id: Option<i32>,
) -> Result<String> {
    let prefix: Option<(String, String)> = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(pool)
    .await?;
    let Some((section, group)) = prefix else {
        return Ok("/".to_string());
    };
    let anchor = comment_id
        .map(|id| format!("?cid={id}"))
        .unwrap_or_default();
    Ok(format!("/{section}/{group}/{topic_id}{anchor}"))
}

#[derive(Debug, Clone)]
struct StReactionSignatureView {
    bAnonymous: bool,
    bBlocked: bool,
    sStarsHtml: String,
    iScore: i32,
    iMaxScore: i32,
    bShowScore: bool,
}

#[derive(Debug, Clone)]
struct StReactionTagView {
    sName: String,
    sUrl: String,
}

#[derive(Template)]
#[template(path = "reaction_topic.html")]
struct StReactionTopicTemplate {
    stTopic: TopicDetail,
    stSignature: StReactionSignatureView,
    vecTags: Vec<StReactionTagView>,
    sTopicHtml: String,
    sImagesHtml: String,
    optPoll: Option<crate::routes::topics::PollView>,
    bLinksAllowed: bool,
    sLinkText: String,
    sReactionsHtml: String,
}

#[derive(Template)]
#[template(path = "reaction_comment.html")]
struct StReactionCommentTemplate {
    stTopic: TopicDetail,
    stComment: CommentItem,
    stSignature: StReactionSignatureView,
    sCommentHtml: String,
    optUserpicUrl: Option<String>,
    iUserpicWidth: i32,
    iUserpicHeight: i32,
    bTopicAuthor: bool,
    sReactionsHtml: String,
}

type TyReactionRow = (
    i32,
    String,
    String,
    i32,
    bool,
    Option<chrono::DateTime<chrono::Utc>>,
);

fn stReactionSignature(
    iScore: i32,
    iMaxScore: i32,
    bRegistered: bool,
    bBlocked: bool,
    bModeratorSession: bool,
) -> StReactionSignatureView {
    let iNormalizedScore = iScore.clamp(0, 599);
    let iNormalizedMaxScore = iMaxScore.max(iScore).clamp(0, 599);
    let iGreenStars = iNormalizedScore / 100;
    let iGreyStars = iNormalizedMaxScore / 100 - iGreenStars;
    StReactionSignatureView {
        bAnonymous: !bRegistered,
        bBlocked,
        sStarsHtml: if bRegistered {
            format!(
                "<span class=\"stars\">{}{}</span>",
                "★".repeat(iGreenStars as usize),
                "☆".repeat(iGreyStars as usize)
            )
        } else {
            String::new()
        },
        iScore,
        iMaxScore,
        bShowScore: bRegistered && bModeratorSession,
    }
}

fn sReactionDateTitle(
    optDate: Option<chrono::DateTime<chrono::Utc>>,
    stTimezone: chrono_tz::Tz,
) -> String {
    let Some(dtValue) = optDate else {
        return String::new();
    };
    let dtLocal = dtValue.with_timezone(&stTimezone);
    let sShortZone = dtLocal.format("%Z").to_string();
    let sZone = if matches!(sShortZone.as_bytes().first(), Some(b'+') | Some(b'-')) {
        format!("GMT{}", dtLocal.format("%:z"))
    } else {
        sShortZone
    };
    format!("{} {sZone}", dtLocal.format("%d.%m.%y %H:%M:%S"))
}

/// Exact `reactions.tag` all-mode used by reaction-topic.jsp and
/// reaction-comment.jsp: every known choice is visible, followed by the
/// chronological author list.  The JSON map remains authoritative; the log
/// only supplies the optional timestamp.
fn sRenderAllReactions(
    iTopicId: i32,
    optCommentId: Option<i32>,
    vecRows: &[TyReactionRow],
    iViewerId: i32,
    bAllowInteract: bool,
    sCsrfToken: &str,
    stTimezone: chrono_tz::Tz,
) -> String {
    let mut vecEmoji = REACTIONS
        .iter()
        .map(|(sEmoji, _)| (*sEmoji).to_owned())
        .collect::<Vec<_>>();
    for (_, _, sReaction, _, _, _) in vecRows {
        if !vecEmoji.contains(sReaction) {
            vecEmoji.push(sReaction.clone());
        }
    }
    // PreparedReactions.allZeros is a TreeMap[String, ...], whose ordering
    // is Java/Scala UTF-16 lexicographic ordering.
    vecEmoji.sort_by_key(|sEmoji| sEmoji.encode_utf16().collect::<Vec<_>>());

    let sDisabled = if bAllowInteract { "" } else { " disabled" };
    let mut sHtml = String::from(
        "<div class=\"reactions \"><form class=\"reactions-form\" action=\"/reactions\" method=\"POST\">",
    );
    sHtml.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"topic\" value=\"{iTopicId}\">",
        html_escape::encode_double_quoted_attribute(sCsrfToken)
    ));
    if let Some(iCommentId) = optCommentId {
        sHtml.push_str(&format!(
            "<input type=\"hidden\" name=\"comment\" value=\"{iCommentId}\">"
        ));
    }

    for sEmoji in vecEmoji {
        let mut vecUsers = vecRows
            .iter()
            .filter(|(_, _, sReaction, _, _, _)| sReaction == &sEmoji)
            .collect::<Vec<_>>();
        vecUsers.sort_by_key(|(_, _, _, iScore, _, _)| std::cmp::Reverse(*iScore));
        let iCount = vecUsers.len();
        let bClicked = vecUsers
            .iter()
            .any(|(iUserId, _, _, _, _, _)| *iUserId == iViewerId);
        let sDescription = REACTIONS
            .iter()
            .find_map(|(sKnown, sDescription)| (*sKnown == sEmoji).then_some(*sDescription))
            .unwrap_or(&sEmoji);
        let sUsers = vecUsers
            .iter()
            .take(3)
            .map(|(_, sNick, _, _, _, _)| sNick.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let sMore = if vecUsers.len() > 3 { "..." } else { "" };
        let sTitle = format!("Реакция \"{sDescription}\": {sUsers}{sMore}");
        let sClickedClass = if bClicked { " btn-primary" } else { "" };
        sHtml.push_str(&format!(
            "<button name=\"reaction\" value=\"{}-{}\" class=\"reaction{sClickedClass} \" title=\"{}\"{sDisabled}>{} <span class=\"reaction-count\">{iCount}</span></button>",
            html_escape::encode_double_quoted_attribute(&sEmoji),
            !bClicked,
            html_escape::encode_double_quoted_attribute(&sTitle),
            html_escape::encode_text(&sEmoji),
        ));
    }
    sHtml.push_str("</form></div><div class=\"reactions\">");
    for (_, sNick, sReaction, _, bBlocked, optDate) in vecRows {
        let sDate = sReactionDateTitle(*optDate, stTimezone);
        let sNickText = html_escape::encode_text(sNick);
        let sNickLink = format!(
            "<a href=\"/people/{}/profile\">{sNickText}</a>",
            urlencoding::encode(sNick)
        );
        let sUser = if *bBlocked {
            format!("<s>{sNickLink}</s>")
        } else {
            sNickLink
        };
        sHtml.push_str(&format!(
            "<span class=\"reaction\" title=\"{}\">{} {sUser}</span>",
            html_escape::encode_double_quoted_attribute(&sDate),
            html_escape::encode_text(sReaction),
        ));
    }
    sHtml.push_str("</div>");
    sHtml
}

async fn vecReactionRows(
    stState: &AppState,
    iTopicId: i32,
    optCommentId: Option<i32>,
    iViewerId: i32,
) -> Result<Vec<TyReactionRow>> {
    let vecRows = if let Some(iCommentId) = optCommentId {
        sqlx::query_as(
            r#"SELECT u.id,u.nick,item.value,COALESCE(u.score,0),
                      COALESCE(u.blocked,false),rl.set_date
               FROM comments c
               CROSS JOIN LATERAL jsonb_each_text(COALESCE(c.reactions,'{}'::jsonb)) item
               JOIN users u ON u.id=item.key::integer
               LEFT JOIN reactions_log rl
                 ON rl.origin_user=u.id AND rl.topic_id=c.topic AND rl.comment_id=c.id
               WHERE c.id=$1 AND item.key ~ '^[0-9]+$'
                 AND NOT EXISTS (
                   SELECT 1 FROM ignore_list il
                   WHERE il.userid=$2 AND il.ignored=u.id
                 )
               ORDER BY COALESCE(rl.set_date,'epoch'::timestamptz)"#,
        )
        .bind(iCommentId)
        .bind(iViewerId)
        .fetch_all(&stState.pool)
        .await?
    } else {
        sqlx::query_as(
            r#"SELECT u.id,u.nick,item.value,COALESCE(u.score,0),
                      COALESCE(u.blocked,false),rl.set_date
               FROM topics t
               CROSS JOIN LATERAL jsonb_each_text(COALESCE(t.reactions,'{}'::jsonb)) item
               JOIN users u ON u.id=item.key::integer
               LEFT JOIN reactions_log rl
                 ON rl.origin_user=u.id AND rl.topic_id=t.id AND rl.comment_id IS NULL
               WHERE t.id=$1 AND item.key ~ '^[0-9]+$'
                 AND NOT EXISTS (
                   SELECT 1 FROM ignore_list il
                   WHERE il.userid=$2 AND il.ignored=u.id
                 )
               ORDER BY COALESCE(rl.set_date,'epoch'::timestamptz)"#,
        )
        .bind(iTopicId)
        .bind(iViewerId)
        .fetch_all(&stState.pool)
        .await?
    };
    Ok(vecRows)
}

async fn optReactionPoll(
    stState: &AppState,
    stTopic: &TopicDetail,
    bExpired: bool,
    bResultsRequested: bool,
    stUser: &UserSummary,
    sCsrfToken: &str,
) -> Result<Option<crate::routes::topics::PollView>> {
    let Some((iPollId, bMultiselect)): Option<(i32, bool)> =
        sqlx::query_as("SELECT id,multiselect FROM polls WHERE topic=$1")
            .bind(stTopic.id)
            .fetch_optional(&stState.pool)
            .await?
    else {
        return Ok(None);
    };
    let mut vecRows: Vec<(i32, String, i32, bool)> = sqlx::query_as(
        r#"SELECT v.id,v.label,v.votes,
                  EXISTS(SELECT 1 FROM vote_users vu
                         WHERE vu.vote=v.vote AND vu.variant_id=v.id AND vu.userid=$2)
           FROM polls_variants v WHERE v.vote=$1 ORDER BY v.id"#,
    )
    .bind(iPollId)
    .bind(stUser.id)
    .fetch_all(&stState.pool)
    .await?;
    let iTotalVotes: i32 = vecRows.iter().map(|(_, _, iVotes, _)| *iVotes).sum();
    let iTotalPeople: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT userid) FROM vote_users WHERE vote=$1")
            .bind(iPollId)
            .fetch_one(&stState.pool)
            .await?;
    let bUserVoted = vecRows.iter().any(|(_, _, _, bVoted)| *bVoted);
    let bPending = !stTopic.moderate;
    let bShowResults = !bPending && (bResultsRequested || bUserVoted || bExpired);
    if bShowResults {
        vecRows.sort_by_key(|(iId, _, iVotes, _)| (std::cmp::Reverse(*iVotes), *iId));
    }
    let iMaxVotes = vecRows
        .iter()
        .map(|(_, _, iVotes, _)| *iVotes)
        .max()
        .unwrap_or(0);
    let iDivisor = if bMultiselect {
        i32::try_from(iTotalPeople)
            .unwrap_or(i32::MAX)
            .max(iMaxVotes)
    } else {
        iTotalVotes
    };
    let vecVariants = vecRows
        .into_iter()
        .map(|(iId, sLabel, iVotes, bVoted)| {
            let iWidth = if iMaxVotes > 0 {
                320 * iVotes / iMaxVotes
            } else {
                0
            };
            crate::routes::topics::PollVariantView {
                id: iId,
                label: sLabel,
                votes: iVotes,
                pct: if iDivisor > 0 {
                    ((100.0 * f64::from(iVotes) / f64::from(iDivisor)).round()) as i32
                } else {
                    0
                },
                progress_pct: (iWidth / 16) * 16 * 100 / 320,
                progress_alt: "*".repeat(iWidth as usize),
                user_voted: bVoted,
            }
        })
        .collect();
    Ok(Some(crate::routes::topics::PollView {
        voteid: iPollId,
        multiselect: bMultiselect,
        variants: vecVariants,
        total_votes: iTotalVotes,
        total_people: iTotalPeople,
        can_vote: !bUserVoted && !stTopic.deleted && !bPending && !bExpired,
        show_results: bShowResults,
        pending: bPending,
        authorized: true,
        topic_url: stTopic.topic_url(),
        csrf_token: sCsrfToken.to_owned(),
    }))
}

fn sRenderReactionImages(
    vecImages: &[crate::routes::topics::TopicImageView],
    sStoredTitle: &str,
    bImagePost: bool,
) -> String {
    let sTitle = crate::domain::title::sTopicTitlePlainForDisplay(sStoredTitle);
    let sTitleAttr = html_escape::encode_double_quoted_attribute(&sTitle);
    match vecImages {
        [] => String::new(),
        [stImage] => {
            let sSrcset = crate::routes::topics::topic_image_srcset(stImage);
            let sOpen = if bImagePost || stImage.width >= 1920 || stImage.height >= 1080 {
                format!(
                    "<a href=\"{}\" itemprop=\"contentURL\">",
                    html_escape::encode_double_quoted_attribute(&stImage.original_url)
                )
            } else {
                String::new()
            };
            let sClose = if sOpen.is_empty() { "" } else { "</a>" };
            format!(
                "<div class=\"medium-image-container\"><figure class=\"medium-image\" itemprop=\"associatedMedia\" itemscope itemtype=\"http://schema.org/ImageObject\">{sOpen}<img itemprop=\"thumbnail\" class=\"medium-image\" src=\"{}\" alt=\"{sTitleAttr}\" srcset=\"{}\" sizes=\"(min-width: 70em) 80vw, 100vw\" width=\"{}\" height=\"{}\">{sClose}<meta itemprop=\"caption\" content=\"{sTitleAttr}\"></figure></div>",
                html_escape::encode_double_quoted_attribute(&stImage.medium_url),
                html_escape::encode_double_quoted_attribute(&sSrcset),
                stImage.width,
                stImage.height,
            )
        }
        _ => {
            let mut sItems = String::new();
            let mut sIndicators = String::new();
            for (iIndex, stImage) in vecImages.iter().enumerate() {
                let sSrcset = crate::routes::topics::topic_image_srcset(stImage);
                sItems.push_str(&format!(
                    "<a href=\"{}\"><img src=\"{}\" alt=\"{sTitleAttr}\" srcset=\"{}\" sizes=\"(min-width: 70em) 80vw, 100vw\" width=\"{}\" height=\"{}\"></a>",
                    html_escape::encode_double_quoted_attribute(&stImage.original_url),
                    html_escape::encode_double_quoted_attribute(&stImage.medium_url),
                    html_escape::encode_double_quoted_attribute(&sSrcset),
                    stImage.width,
                    stImage.height,
                ));
                sIndicators.push_str(&format!(
                    "<a href=\"{}\"{}></a>",
                    html_escape::encode_double_quoted_attribute(&stImage.original_url),
                    if iIndex == 0 { " class=\"active\"" } else { "" }
                ));
            }
            format!(
                "<div class=\"slider-parent\"><div class=\"swiffy-slider slider-indicators-round slider-indicators-outside slider-indicators-sm slider-item-ratio slider-item-ratio-contain\"><div class=\"slider-container\">{sItems}</div><button type=\"button\" class=\"slider-nav\" aria-label=\"Предыдущее изображение\"></button><button type=\"button\" class=\"slider-nav slider-nav-next\" aria-label=\"Следующее изображение\"></button><div class=\"slider-indicators\">{sIndicators}</div></div></div>"
            )
        }
    }
}

/// ReactionController.commentReaction/topicReaction (GET, non-ajax): an
/// anonymous visitor is redirected straight to the topic/comment; a logged
/// in user gets an HTML breakdown of who reacted with what. The previous
/// handler always returned raw JSON regardless of auth state or Accept
/// header, which isn't what a plain browser GET (e.g. from a bookmarked
/// link or the non-JS reaction UI) expects.
pub async fn reactions_get(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    jar: CookieJar,
    Query(q): Query<ReactionQuery>,
) -> Result<axum::response::Response> {
    // Spring chooses the `params=comment` mapping whenever `comment` is
    // present, ignoring a simultaneous `topic` value.  Without `comment`,
    // `topic` is required; the non-original `msgid` alias is deliberately
    // not accepted for GET.
    let (topic_id, comment_id) = if let Some(iCommentId) = q.comment {
        let iTopicId: i32 = sqlx::query_scalar("SELECT topic FROM comments WHERE id=$1")
            .bind(iCommentId)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
        (iTopicId, Some(iCommentId))
    } else {
        (
            q.topic
                .ok_or_else(|| AppError::BadRequest("missing topic".into()))?,
            None,
        )
    };
    // Java loads the target before checking whether a session exists, so an
    // unknown id is a 404 rather than an anonymous redirect to `/`.
    let stTopic = crate::routes::topics::get_topic(&state, topic_id).await?;
    let sLink = comment_id.map_or_else(
        || stTopic.topic_url(),
        |iCommentId| format!("{}?cid={iCommentId}", stTopic.topic_url()),
    );

    if user.is_none() {
        return Ok((StatusCode::FOUND, [(header::LOCATION, sLink)]).into_response());
    }
    let stUser = user.as_ref().expect("authorized above");

    // ReactionController.commentReaction/topicReaction: a deleted
    // topic/comment (or a topic with comments hidden) isn't viewable even
    // by an authorized user; a plain topic view additionally runs the full
    // checkView gate (deleted/draft/expired/open-warnings visibility).
    if let Some(comment_id) = comment_id {
        let (comment_deleted, topic_deleted, topic_postscore): (bool, bool, i32) = sqlx::query_as(
            "SELECT c.deleted, t.deleted, COALESCE(t.postscore, -9999) FROM comments c JOIN topics t ON t.id=c.topic WHERE c.id=$1",
        )
        .bind(comment_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
        if comment_deleted || topic_deleted || topic_postscore == POSTSCORE_HIDE_COMMENTS {
            return Err(AppError::Forbidden);
        }
    } else {
        crate::routes::topics::check_topic_viewable(&state, topic_id, &user).await?;
        if stTopic.deleted {
            return Err(AppError::Forbidden);
        }
    }

    let (bExpired, iPostscore, bLinksAllowed, bPollAllowed): (bool, i32, bool, bool) =
        sqlx::query_as(
            r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate)+s.expire<CURRENT_TIMESTAMP,
                      COALESCE(t.postscore,-9999),s.havelink,COALESCE(s.vote,false)
               FROM topics t JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
        )
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;
    let bFrozen = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT frozen_until FROM users WHERE id=$1",
    )
    .bind(stUser.id)
    .fetch_one(&state.pool)
    .await?
    .is_some_and(|dtUntil| dtUntil > chrono::Utc::now());
    let vecRows = vecReactionRows(&state, topic_id, comment_id, stUser.id).await?;
    let stTimezone = crate::request_timezone::stRequestTimezone(&jar);

    if let Some(iCommentId) = comment_id {
        let stComment: CommentItem = sqlx::query_as(
            r#"SELECT c.id,c.topic,c.replyto,c.title,m.message,m.markup::text AS markup,
                      c.postdate,u.id AS author_id,u.nick AS author,
                      COALESCE(u.score,0) AS author_score,
                      COALESCE(u.blocked,false) AS author_blocked,
                      COALESCE(u.passwd,'')='' AS author_anonymous,
                      COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS author_frozen,
                      c.deleted
               FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
               WHERE c.id=$1"#,
        )
        .bind(iCommentId)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        let (iScore, iMaxScore, bRegistered, bBlocked, optPhoto, optEmail): (
            i32,
            i32,
            bool,
            bool,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            r#"SELECT COALESCE(score,0),COALESCE(max_score,0),COALESCE(passwd,'')<>'',
                      COALESCE(blocked,false),photo,email FROM users WHERE id=$1"#,
        )
        .bind(stComment.author_id)
        .fetch_one(&state.pool)
        .await?;
        let optSettings: Option<String> =
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(stUser.id)
                .fetch_optional(&state.pool)
                .await?;
        let stProfile = crate::profile::ProfileSettings::from_hstore_text(optSettings);
        let (optUserpicUrl, iUserpicWidth, iUserpicHeight) = if stProfile.photos {
            let stUserpic = crate::profile::stResolveUserpic(
                std::path::Path::new(&state.config.upload_dir),
                &stProfile.avatar,
                false,
                stComment.author_id == 2,
                optPhoto.as_deref(),
                optEmail.as_deref(),
            );
            (Some(stUserpic.sUrl), stUserpic.iWidth, stUserpic.iHeight)
        } else {
            (None, 0, 0)
        };
        let bAllowInteract = !bFrozen
            && !bExpired
            && !stComment.deleted
            && iPostscore != 10002
            && stUser.id != stComment.author_id;
        let sReactionsHtml = sRenderAllReactions(
            topic_id,
            Some(iCommentId),
            &vecRows,
            stUser.id,
            bAllowInteract,
            &csrf_token,
            stTimezone,
        );
        let stMarkupUsers = state
            .markup
            .stResolveBatch([(&*stComment.message, &*stComment.markup)])
            .await?;
        return Ok(Html(
            StReactionCommentTemplate {
                stSignature: stReactionSignature(
                    iScore,
                    iMaxScore,
                    bRegistered,
                    bBlocked,
                    stUser.canmod,
                ),
                sCommentHtml: markup::render_message_with_markup_policy_and_users(
                    &stComment.message,
                    Some(&stComment.markup),
                    None,
                    stComment.bNofollowAuthorLinks(),
                    Some(&state.config.public_url),
                    Some(&stMarkupUsers),
                ),
                optUserpicUrl,
                iUserpicWidth,
                iUserpicHeight,
                bTopicAuthor: stComment.author_id == stTopic.author_id,
                sReactionsHtml,
                stTopic,
                stComment,
            }
            .render()?,
        )
        .into_response());
    }

    let (iScore, iMaxScore, bRegistered, bBlocked): (i32, i32, bool, bool) = sqlx::query_as(
        r#"SELECT COALESCE(score,0),COALESCE(max_score,0),COALESCE(passwd,'')<>'',
                      COALESCE(blocked,false) FROM users WHERE id=$1"#,
    )
    .bind(stTopic.author_id)
    .fetch_one(&state.pool)
    .await?;
    let bAllowInteract = !bFrozen && !bExpired && stUser.id != stTopic.author_id;
    let sReactionsHtml = sRenderAllReactions(
        topic_id,
        None,
        &vecRows,
        stUser.id,
        bAllowInteract,
        &csrf_token,
        stTimezone,
    );
    let vecImages = crate::routes::topics::load_topic_images(&state, topic_id).await?;
    let sImagesHtml = sRenderReactionImages(
        &vecImages,
        &stTopic.title,
        stTopic.section_prefix == "gallery",
    );
    let optPoll = if bPollAllowed {
        optReactionPoll(
            &state,
            &stTopic,
            bExpired,
            q.results.as_deref() == Some("true"),
            stUser,
            &csrf_token,
        )
        .await?
    } else {
        None
    };
    let vecTags = stTopic
        .tags_vec()
        .into_iter()
        .map(|sName| StReactionTagView {
            sUrl: format!("/tag/{}", urlencoding::encode(&sName)),
            sName,
        })
        .collect();
    let sLinkText = stTopic
        .linktext
        .as_deref()
        .filter(|sValue| !sValue.is_empty())
        .unwrap_or("Подробности")
        .to_owned();
    let stMarkupUsers = state
        .markup
        .stResolveBatch([(&*stTopic.message, &*stTopic.markup)])
        .await?;
    Ok(Html(
        StReactionTopicTemplate {
            stSignature: stReactionSignature(
                iScore,
                iMaxScore,
                bRegistered,
                bBlocked,
                stUser.canmod,
            ),
            vecTags,
            sTopicHtml: markup::render_topic_with_expanded_cut_policy_and_users(
                &stTopic.message,
                &stTopic.markup,
                stTopic.bNofollowAuthorLinks(),
                Some(&state.config.public_url),
                Some(&stMarkupUsers),
            ),
            sImagesHtml,
            optPoll,
            bLinksAllowed,
            sLinkText,
            sReactionsHtml,
            stTopic,
        }
        .render()?,
    )
    .into_response())
}

/// ReactionService.DefinedReactions - order matters (matches insertion order
/// in the Java `Map`, which the JSP iterates for button layout).
pub(crate) const REACTIONS: &[(&str, &str)] = &[
    ("👍", "большой палец вверх"),
    ("👎", "большой палец вниз"),
    ("😊", "улыбающееся лицо"),
    ("😱", "лицо, кричащее от страха"),
    ("🤦", "facepalm"),
    ("🔥", "огонь"),
    ("🤔", "задумчивое лицо"),
    ("🤡", "клоунада"),
    ("☕☕", "два чая этому господину!"),
    ("🪗", "боян!!!1111"),
    ("😢", "грусть-печаль"),
    ("🚮", "не нужно!"),
    ("🎉", "хлопушка"),
    ("🤬", "нет слов!"),
];

#[cfg(test)]
mod reaction_get_contract_tests {
    use super::*;
    use axum::http::HeaderValue;
    use chrono::TimeZone;

    #[test]
    fn all_mode_renders_every_choice_and_author_log_like_reactions_tag() {
        let dtSet = chrono::Utc
            .with_ymd_and_hms(2026, 8, 15, 9, 10, 11)
            .unwrap();
        let sHtml = sRenderAllReactions(
            42,
            Some(7),
            &[
                (10, "alice".into(), "🎉".into(), 300, false, Some(dtSet)),
                (11, "blocked".into(), "custom".into(), 50, true, None),
            ],
            10,
            true,
            "csrf-token",
            chrono_tz::Europe::Moscow,
        );

        assert_eq!(
            sHtml.matches("<button name=\"reaction\"").count(),
            REACTIONS.len() + 1
        );
        assert!(sHtml.contains("method=\"POST\""));
        assert!(sHtml.contains("name=\"comment\" value=\"7\""));
        assert!(sHtml.contains("value=\"🎉-false\" class=\"reaction btn-primary \""));
        assert!(sHtml.contains("value=\"custom-true\""));
        assert!(sHtml.contains("title=\"15.08.26 12:10:11 MSK\""));
        assert!(sHtml.contains("<s><a href=\"/people/blocked/profile\">blocked</a></s>"));
        assert!(!sHtml.contains("Нет реакций"));
    }

    #[test]
    fn disabled_all_mode_keeps_buttons_visible_but_non_interactive() {
        let sHtml =
            sRenderAllReactions(42, None, &[], 10, false, "csrf-token", chrono_tz::Etc::UTC);

        assert_eq!(
            sHtml.matches("<button name=\"reaction\"").count(),
            REACTIONS.len()
        );
        assert_eq!(sHtml.matches(" disabled>").count(), REACTIONS.len());
        assert!(!sHtml.contains("reaction-show-list"));
        assert!(!sHtml.contains("reaction-show\""));
    }

    #[test]
    fn reaction_pages_use_full_base_layout_and_original_return_dom() {
        let sTopic = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/reaction_topic.html"
        ));
        let sComment = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/reaction_comment.html"
        ));
        for sTemplate in [sTopic, sComment] {
            assert!(sTemplate.contains("{% extends \"base.html\" %}"));
            assert!(sTemplate.contains("class=\"messages\""));
            assert!(sTemplate.contains("class=\"btn btn-primary\""));
            assert!(sTemplate.contains("Вернуться"));
        }
        assert!(sTopic.contains("id=\"topic-{{ stTopic.id }}\""));
        assert!(sComment.contains("id=\"comment-{{ stComment.id }}\""));
        assert!(sComment.contains("class=\"userpic\""));
        assert!(!sTopic.contains("<h1>Реакции</h1>"));
    }

    #[test]
    fn anonymous_redirect_contract_is_http_302() {
        let stResponse = (
            StatusCode::FOUND,
            [(header::LOCATION, "/forum/linux-org-ru/42".to_owned())],
        )
            .into_response();
        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse.headers().get(header::LOCATION),
            Some(&HeaderValue::from_static("/forum/linux-org-ru/42"))
        );
    }
}

async fn check_reaction_allowed(
    pool: &sqlx::PgPool,
    user_id: i32,
    topic_id: i32,
    comment_id: Option<i32>,
    set: bool,
    reaction: &str,
) -> Result<()> {
    if !REACTIONS.iter().any(|(r, _)| *r == reaction) {
        return Err(crate::error::AppError::Forbidden);
    }
    if set {
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM reactions_log WHERE origin_user=$1 AND set_date > CURRENT_TIMESTAMP - interval '10 minutes'",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        if recent >= 5 {
            return Err(crate::error::AppError::TooManyRequests(
                "Попробуйте позже".into(),
            ));
        }
    }

    let (author_id, topic_deleted, topic_expired, comment_deleted, topic_postscore): (
        i32,
        bool,
        bool,
        Option<bool>,
        i32,
    ) = if let Some(comment_id) = comment_id {
        let row: (i32, bool, bool, bool, i32) = sqlx::query_as(
            r#"SELECT c.userid,
                      t.deleted,
                      NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired,
                      c.deleted,
                      COALESCE(t.postscore, -9999)
               FROM comments c
               JOIN topics t ON t.id=c.topic
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               WHERE c.id=$1 AND t.id=$2"#,
        )
        .bind(comment_id)
        .bind(topic_id)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::AppError::NotFound)?;
        (row.0, row.1, row.2, Some(row.3), row.4)
    } else {
        let (author_id, deleted, expired, postscore): (i32, bool, bool, i32) = sqlx::query_as(
            r#"SELECT t.userid, t.deleted, NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired, COALESCE(t.postscore, -9999)
               FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
        )
        .bind(topic_id)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::AppError::NotFound)?;
        (author_id, deleted, expired, None, postscore)
    };

    // ReactionService.allowInteract: comment reactions are additionally
    // blocked once the topic's comments are hidden (POSTSCORE_HIDE_COMMENTS).
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    let comments_hidden = comment_id.is_some() && topic_postscore == POSTSCORE_HIDE_COMMENTS;

    if user_id == author_id
        || topic_deleted
        || topic_expired
        || comment_deleted.unwrap_or(false)
        || comments_hidden
    {
        return Err(crate::error::AppError::Forbidden);
    }

    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Err(crate::error::AppError::Forbidden);
    }

    Ok(())
}

struct SetReactionResult {
    topic_id: i32,
    comment_id: Option<i32>,
    count: i64,
}

async fn do_set_reaction(
    state: &AppState,
    user_id: i32,
    form: ReactionForm,
) -> Result<SetReactionResult> {
    let (topic_id, comment_id) =
        resolve_reaction_target(&state.pool, form.topic, form.comment, form.msgid).await?;
    let (reaction, set) = parse_reaction_action(form.reaction, form.value);
    check_reaction_allowed(&state.pool, user_id, topic_id, comment_id, set, &reaction).await?;

    let mut tx = state.pool.begin().await?;
    let reactions: serde_json::Value = if set {
        let updated_reactions = if let Some(comment_id) = comment_id {
            sqlx::query_scalar("UPDATE comments SET reactions=reactions || jsonb_build_object($2::text,$3::text) WHERE id=$1 RETURNING reactions")
                .bind(comment_id).bind(user_id).bind(&reaction).fetch_one(&mut *tx).await?
        } else {
            sqlx::query_scalar("UPDATE topics SET reactions=reactions || jsonb_build_object($2::text,$3::text) WHERE id=$1 RETURNING reactions")
                .bind(topic_id).bind(user_id).bind(&reaction).fetch_one(&mut *tx).await?
        };
        sqlx::query(
            r#"INSERT INTO reactions_log(origin_user,topic_id,comment_id,reaction,set_date)
               VALUES($1,$2,$3,$4,now())
               ON CONFLICT (topic_id, comment_id, origin_user)
               DO UPDATE SET set_date=now(), reaction=EXCLUDED.reaction"#,
        )
        .bind(user_id)
        .bind(topic_id)
        .bind(comment_id)
        .bind(&reaction)
        .execute(&mut *tx)
        .await?;
        updated_reactions
    } else {
        let reactions = if let Some(comment_id) = comment_id {
            sqlx::query_scalar(
                "UPDATE comments SET reactions=reactions-$2::text WHERE id=$1 RETURNING reactions",
            )
            .bind(comment_id)
            .bind(user_id.to_string())
            .fetch_one(&mut *tx)
            .await?
        } else {
            sqlx::query_scalar(
                "UPDATE topics SET reactions=reactions-$2::text WHERE id=$1 RETURNING reactions",
            )
            .bind(topic_id)
            .bind(user_id.to_string())
            .fetch_one(&mut *tx)
            .await?
        };
        sqlx::query(
            r#"DELETE FROM reactions_log
               WHERE origin_user=$1 AND topic_id=$2 AND (($3::int IS NULL AND comment_id IS NULL) OR comment_id=$3)"#,
        )
        .bind(user_id).bind(topic_id).bind(comment_id).execute(&mut *tx).await?;
        reactions
    };

    // ReactionService updates topic ordering for both topic and comment
    // reactions, and manages the target author's unread notification.
    sqlx::query("UPDATE topics SET lastmod=now() WHERE id=$1")
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    let target_user: i32 = if let Some(comment_id) = comment_id {
        sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1")
            .bind(comment_id)
            .fetch_one(&mut *tx)
            .await?
    } else {
        sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?
    };
    if set {
        let settings_text: Option<String> =
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(target_user)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        let notify =
            crate::profile::ProfileSettings::from_hstore_text(settings_text).reaction_notification;
        let ignored: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM ignore_list WHERE userid=$1 AND ignored=$2)",
        )
        .bind(target_user)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if notify && !ignored && target_user != crate::routes::comments::ANONYMOUS_USER_ID {
            sqlx::query(
                r#"INSERT INTO user_events(userid,type,private,message_id,comment_id,origin_user)
                   VALUES($1,'REACTION',false,$2,$3,$4) ON CONFLICT DO NOTHING"#,
            )
            .bind(target_user)
            .bind(topic_id)
            .bind(comment_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
    } else {
        sqlx::query(
            r#"DELETE FROM user_events WHERE userid=$1 AND message_id=$2
               AND comment_id IS NOT DISTINCT FROM $3 AND origin_user=$4 AND unread AND type='REACTION'"#,
        ).bind(target_user).bind(topic_id).bind(comment_id).bind(user_id).execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE users SET unread_events=(SELECT count(*) FROM user_events WHERE userid=$1 AND unread) WHERE id=$1",
    ).bind(target_user).execute(&mut *tx).await?;

    let count = reactions
        .as_object()
        .map(|values| {
            values
                .values()
                .filter(|value| value.as_str() == Some(&reaction))
                .count() as i64
        })
        .unwrap_or(0);
    tx.commit().await?;
    state.realtime.vNotifyEvents([target_user]);

    Ok(SetReactionResult {
        topic_id,
        comment_id,
        count,
    })
}

/// ReactionController.setCommentReaction/setTopicReaction (POST, non-ajax
/// form submit) - redirects back to the topic/comment, matching Java's
/// RedirectView. The previous handler always returned JSON here too, which
/// breaks a plain `<form method=post>` submit (no fetch/XHR).
pub async fn reactions_post(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Form(form): axum::Form<ReactionForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let result = do_set_reaction(&state, user.id, form).await?;
    let link = reaction_target_link(&state.pool, result.topic_id, result.comment_id).await?;
    Ok(Redirect::to(&link))
}

/// ReactionController.setCommentReactionAjax/setTopicReactionAjax (POST /reactions/ajax).
pub async fn reactions_post_ajax(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Form(form): axum::Form<ReactionForm>,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let result = do_set_reaction(&state, user.id, form).await?;
    Ok(Json(json!({"count": result.count})))
}

pub struct VoteForm {
    /// Poll id (`voteid` in the original VoteController).
    pub voteid: i32,
    /// Selected variant ids. The original form submits this field as repeated `vote`.
    pub vote: Vec<i32>,
}

/// `VoteController` rejects `!msg.commited`; `Topic.commited` is read
/// directly from `topics.moderate` in the Java model.
const VOTE_TOPIC_SQL: &str = r#"SELECT p.topic, p.multiselect,
          CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
          g.urlname,
          NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired
   FROM polls p
   JOIN topics t ON t.id=p.topic
   JOIN groups g ON g.id=t.groupid
   JOIN sections s ON s.id=g.section
   WHERE p.id=$1 AND t.moderate AND NOT t.deleted AND NOT t.draft"#;

pub async fn vote(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    body: axum::body::Bytes,
) -> Result<axum::response::Redirect> {
    let Some(user) = user else {
        return Err(crate::error::AppError::Forbidden);
    };
    let pairs = crate::form::parse_pairs(&body)?;
    let form = VoteForm {
        voteid: crate::form::get(&pairs, "voteid")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| AppError::BadRequest("missing voteid".into()))?,
        vote: crate::form::get_all(&pairs, "vote")
            .into_iter()
            .filter_map(|v| v.parse().ok())
            .collect(),
    };
    if form.vote.is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "ничего не выбрано".into(),
        ));
    }

    let Some((topic_id, multiselect, section_prefix, group_urlname, expired)) =
        sqlx::query_as::<_, (i32, bool, String, String, bool)>(VOTE_TOPIC_SQL)
            .bind(form.voteid)
            .fetch_optional(&state.pool)
            .await?
    else {
        return Err(crate::error::AppError::BadRequest(
            "опрос не найден или ещё не подтверждён".into(),
        ));
    };

    if expired {
        return Err(crate::error::AppError::BadRequest("Опрос завершен".into()));
    }
    if !multiselect && form.vote.len() != 1 {
        return Err(crate::error::AppError::BadRequest(
            "этот опрос допускает только один вариант ответа".into(),
        ));
    }

    let mut selected = form.vote;
    selected.sort_unstable();
    selected.dedup();
    let valid_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM polls_variants WHERE vote=$1 AND id = ANY($2)")
            .bind(form.voteid)
            .bind(&selected)
            .fetch_one(&state.pool)
            .await?;
    if valid_count != selected.len() as i64 {
        return Err(crate::error::AppError::BadRequest(
            "неправильный вариант ответа".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let already_voted: i64 =
        sqlx::query_scalar("SELECT count(vote) FROM vote_users WHERE vote=$1 AND userid=$2")
            .bind(form.voteid)
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if already_voted == 0 {
        for variant_id in selected {
            let inserted = sqlx::query(
                "INSERT INTO vote_users(vote, userid, variant_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
            )
            .bind(form.voteid)
            .bind(user.id)
            .bind(variant_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted > 0 {
                sqlx::query("UPDATE polls_variants SET votes=votes+1 WHERE id=$1 AND vote=$2")
                    .bind(variant_id)
                    .bind(form.voteid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;

    Ok(axum::response::Redirect::to(&format!(
        "/{section_prefix}/{group_urlname}/{topic_id}"
    )))
}

#[cfg(test)]
mod moderation_semantics_tests {
    use super::{
        NotificationEvent, TRACKER_PUBLIC_TOPICS_CLAUSE, UNCOMMITTED_COUNTS_SQL, VOTE_TOPIC_SQL,
        bNotificationIsCurrent, sNotificationDetails, sTrackerOldLocation, sUnreadDescription,
        tracker_commit_visibility_clause, vecPrepareNotifications,
    };

    fn stNotification(
        iId: i32,
        iSeconds: i64,
        sType: &str,
        iTopicId: i32,
        optCommentId: Option<i32>,
        bUnread: bool,
        optReaction: Option<&str>,
        optNick: Option<&str>,
    ) -> NotificationEvent {
        NotificationEvent {
            id: iId,
            event_date: chrono::DateTime::from_timestamp(iSeconds, 0).unwrap(),
            subj: "topic".into(),
            msgid: iTopicId,
            cid: optCommentId,
            unread: bUnread,
            event_type: sType.into(),
            section_prefix: "forum".into(),
            section_name: "Форум".into(),
            group_urlname: "test".into(),
            origin_nick: optNick.map(str::to_owned),
            author_nick: optNick.unwrap_or("author").to_owned(),
            event_message: None,
            closed_warning: false,
            bonus: None,
            tags: vec!["linux".into()],
            message_text: "body".into(),
            message_markup: "MARKDOWN".into(),
            reaction: optReaction.map(str::to_owned),
        }
    }

    #[test]
    fn removed_reaction_event_is_hidden_only_from_the_main_notification_view() {
        let stRemoved = stNotification(1, 1, "REACTION", 10, None, false, None, Some("alice"));
        let stReply = stNotification(2, 2, "REPLY", 10, None, true, None, None);

        assert!(!bNotificationIsCurrent(&stRemoved));
        assert!(bNotificationIsCurrent(&stReply));
    }

    #[test]
    fn notification_html_subject_is_plain_but_raw_storage_value_is_preserved() {
        let mut stEvent = stNotification(1, 1, "REPLY", 10, None, true, None, None);
        stEvent.subj = "A &amp; B &lt;b&gt; &quot;Q&quot; &#39;X&#39;".to_owned();

        assert_eq!(stEvent.sSubjectPlain(), "A & B <b> «Q» 'X'");
        assert_eq!(
            stEvent.subj,
            "A &amp; B &lt;b&gt; &quot;Q&quot; &#39;X&#39;"
        );
    }

    #[test]
    fn new_notification_design_groups_reactions_by_target_and_read_state() {
        let vecEvents = vec![
            stNotification(3, 30, "REACTION", 20, None, true, Some("🔥"), Some("carol")),
            stNotification(
                2,
                20,
                "REACTION",
                10,
                Some(7),
                true,
                Some("🎉"),
                Some("bob"),
            ),
            stNotification(
                1,
                10,
                "REACTION",
                10,
                Some(7),
                true,
                Some("👍"),
                Some("alice"),
            ),
        ];

        let vecPrepared = vecPrepareNotifications(vecEvents, true);

        assert_eq!(vecPrepared.len(), 2);
        let stGrouped = vecPrepared
            .iter()
            .find(|stItem| stItem.stEvent.msgid == 10)
            .unwrap();
        assert_eq!(stGrouped.stEvent.id, 1);
        assert_eq!(stGrouped.iLastId, 2);
        assert_eq!(
            stGrouped.vecReactions,
            vec![("👍".into(), "alice".into()), ("🎉".into(), "bob".into())]
        );
    }

    #[test]
    fn old_notification_design_keeps_reactions_as_separate_rows() {
        let vecEvents = vec![
            stNotification(2, 20, "REACTION", 10, None, true, Some("🎉"), Some("bob")),
            stNotification(1, 10, "REACTION", 10, None, true, Some("👍"), Some("alice")),
        ];

        assert_eq!(vecPrepareNotifications(vecEvents, false).len(), 2);
    }

    #[test]
    fn deleted_comment_uses_original_dedicated_view() {
        let stEvent = stNotification(1, 1, "DEL", 10, Some(77), true, None, None);
        assert_eq!(stEvent.link(), "/view-deleted?id=77#comment-77");
    }

    #[test]
    fn warning_and_delete_details_preserve_payload_and_closed_state() {
        let mut stWarning = stNotification(1, 1, "WARNING", 10, None, true, None, None);
        stWarning.event_message = Some("[Нарушение правил] 4.1".into());
        stWarning.closed_warning = true;
        assert_eq!(
            sNotificationDetails(&stWarning),
            "<s>[Нарушение правил] 4.1</s>"
        );

        let mut stDeleted = stNotification(2, 2, "DEL", 10, Some(77), true, None, None);
        stDeleted.event_message = Some("Офтопик <script>".into());
        stDeleted.bonus = Some(-2);
        assert_eq!(
            sNotificationDetails(&stDeleted),
            "Офтопик &lt;script&gt; (-2)"
        );
    }

    #[test]
    fn unread_counter_uses_original_russian_forms() {
        assert_eq!(sUnreadDescription(1), "У вас 1 непрочитанное уведомление");
        assert_eq!(sUnreadDescription(3), "У вас 3 непрочитанных уведомления");
        assert_eq!(sUnreadDescription(12), "У вас 12 непрочитанных уведомлений");
        assert_eq!(sUnreadDescription(21), "У вас 21 непрочитанное уведомление");
    }

    #[test]
    fn tracker_hides_only_uncommitted_premoderated_topics() {
        assert_eq!(
            tracker_commit_visibility_clause(false),
            "AND (t.moderate OR NOT s.moderate)"
        );
        assert_eq!(
            tracker_commit_visibility_clause(false),
            TRACKER_PUBLIC_TOPICS_CLAUSE
        );
        assert_eq!(tracker_commit_visibility_clause(true), "");
    }

    #[test]
    fn legacy_tracker_redirect_uses_the_profile_default_like_java() {
        assert_eq!(sTrackerOldLocation(None, "main"), "/tracker/?filter=all");
        assert_eq!(sTrackerOldLocation(Some("main"), "main"), "/tracker/");
        assert_eq!(sTrackerOldLocation(Some("all"), "all"), "/tracker/");
        assert_eq!(
            sTrackerOldLocation(Some("main"), "all"),
            "/tracker/?filter=main"
        );
        assert_eq!(
            sTrackerOldLocation(Some("invalid value"), "main"),
            "/tracker/?filter=invalid%20value"
        );
    }

    #[test]
    fn queue_counts_only_recent_uncommitted_premoderated_topics() {
        assert!(UNCOMMITTED_COUNTS_SQL.contains("s.moderate AND NOT t.moderate"));
        assert!(UNCOMMITTED_COUNTS_SQL.contains("'3 month'::interval"));
    }

    #[test]
    fn vote_lookup_requires_a_committed_topic() {
        assert!(VOTE_TOPIC_SQL.contains("p.id=$1 AND t.moderate AND NOT t.deleted"));
        assert!(!VOTE_TOPIC_SQL.contains("AND NOT t.moderate"));
    }
}
