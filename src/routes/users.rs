use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    models::{PagerQuery, TopicSummary, UserSummary},
    pagination::Pager,
    profile::{ChoiceOption, NumberOption, ProfileSettings, ThemeOption},
    request_timezone::stRequestTimezone,
    security,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json,
    extract::{ConnectInfo, Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use std::{collections::HashMap, net::SocketAddr, sync::OnceLock};

#[derive(Debug, Clone, sqlx::FromRow)]
struct UserProfileData {
    id: i32,
    nick: String,
    name: Option<String>,
    score: i32,
    max_score: i32,
    photo: Option<String>,
    town: Option<String>,
    userinfo: Option<String>,
    url: Option<String>,
    email: Option<String>,
    canmod: bool,
    candel: bool,
    anonymous: bool,
    corrector: bool,
    blocked: bool,
    activated: bool,
    regdate: Option<chrono::DateTime<chrono::Utc>>,
    lastlogin: Option<chrono::DateTime<chrono::Utc>>,
    userinfo_markup: Option<String>,
}

impl UserProfileData {
    fn status(&self) -> &'static str {
        if self.blocked {
            "заблокирован"
        } else if !self.activated {
            "не активирован"
        } else if self.score >= 100 {
            "активный пользователь"
        } else {
            "новый пользователь"
        }
    }
}

#[derive(Debug, Clone)]
struct UserStats {
    topic_count: i64,
    comment_count: i64,
    first_topic: Option<chrono::DateTime<chrono::Utc>>,
    last_topic: Option<chrono::DateTime<chrono::Utc>>,
    first_comment: Option<chrono::DateTime<chrono::Utc>>,
    last_comment: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
struct BanInfo {
    bandate: chrono::DateTime<chrono::Utc>,
    reason: String,
    moderator_nick: String,
}

#[derive(Debug, Clone)]
struct UserLogEntry {
    action: String,
    action_date: chrono::DateTime<chrono::Utc>,
    actor_nick: String,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    profile: UserProfileData,
    stats: UserStats,
    favorite_tags: Vec<String>,
    ignore_tags: Vec<String>,
    drafts_count: i64,
    is_owner: bool,
    is_moderator: bool,
    can_view_private: bool,
    /// Pre-rendered, sanitized HTML for `profile.userinfo` - see
    /// `render_profile`. Never render `profile.userinfo` directly with
    /// `|safe`; it's raw user input.
    userinfo_html: Option<String>,
    ban_info: Option<BanInfo>,
    frozen_until: Option<chrono::DateTime<chrono::Utc>>,
    is_frozen: bool,
    long_freeze_durations: bool,
    blockable: bool,
    freezable: bool,
    other_accounts: Vec<String>,
    user_log: Vec<UserLogEntry>,
    invited_users: Vec<String>,
    lastlogin_fuzzy: Option<String>,
    show_url: bool,
    show_userinfo: bool,
    url_nofollow: bool,
    remark: Option<String>,
    can_remark: bool,
    ignored: bool,
    can_ignore: bool,
    has_remarks: bool,
    can_load_userpic: bool,
    watch_present: bool,
    fav_present: bool,
    slow_mode: bool,
    slow_mode_reason: String,
    freezer_nick: Option<String>,
    freezing_reason: Option<String>,
    /// `UserService.getUserpic(user, viewer.avatarMode, misteryMan=true)` -
    /// always renders as an `<img>`, falling back to a 1x1 transparent gif
    /// (`DisabledUserpic`) rather than a "no photo" box when the viewer has
    /// avatars disabled or the target has neither a local photo nor email.
    userpic_url: String,
    year_stats_url: String,
    year_stats_user: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    user: UserSummary,
    settings: ProfileSettings,
    hide_adsense_disabled: bool,
    themes: Vec<ThemeOption>,
    avatars: Vec<ChoiceOption>,
    tracker_modes: Vec<ChoiceOption>,
    format_modes: Vec<ChoiceOption>,
    topic_values: Vec<NumberOption>,
    message_values: Vec<NumberOption>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "edit_profile.html")]
struct EditProfileTemplate {
    user: UserSummary,
    profile: UserProfileData,
    can_load_userpic: bool,
    can_edit_info: bool,
    can_edit_info_reason: String,
    info_markup_form_id: String,
    info_markup_title: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "usermod_reset_confirmation.html")]
struct StResetPasswordConfirmationTemplate {
    iUserId: i32,
    sNick: String,
    sProfileLink: String,
    sCsrfToken: String,
}

#[derive(Debug, Clone)]
struct UserSectionLink {
    id: i32,
    name: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "user_topics.html")]
struct UserTopicsTemplate {
    title: String,
    nick: String,
    topics: Vec<crate::routes::topics::NewsTopicView>,
    sections: Vec<UserSectionLink>,
    all_selected: bool,
    prev_link: Option<String>,
    next_link: Option<String>,
}

#[derive(Deserialize)]
pub struct UserTopicFeedQuery {
    pub offset: Option<i64>,
    pub section: Option<i32>,
    pub output: Option<String>,
}

/// UserTopicListController.showUserTopics: `/people/{nick}` (bare, no
/// suffix) is the user's topic feed, a distinct page from the profile at
/// `/people/{nick}/profile` - the previous handler aliased this straight to
/// the profile page. Optional `?section=` filter, 404s if the feed is
/// empty (matches Java exactly, including on a valid user with zero posts).
pub async fn topic_feed(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<UserTopicFeedQuery>,
    current: CurrentUser,
) -> Result<Response> {
    if q.output.as_deref() == Some("rss") {
        return Ok(StatusCode::GONE.into_response());
    }
    let user = get_user(&state, &nick).await?;
    if user.id == crate::routes::comments::ANONYMOUS_USER_ID
        && !current.0.as_ref().is_some_and(|stUser| stUser.canmod)
    {
        return Err(AppError::BadRequest(
            "Лента для пользователя anonymous не доступна".into(),
        ));
    }
    let pager = Pager::new(q.offset.unwrap_or(0).max(0), state.config.page_size);

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE u.id=$1 AND NOT t.deleted AND NOT COALESCE(t.draft,false)
             AND (t.moderate OR NOT s.moderate)
             {section_clause}
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
        section_clause = if q.section.is_some() {
            "AND s.id=$4"
        } else {
            ""
        },
    );
    let mut query = sqlx::query_as::<_, TopicSummary>(sqlx::AssertSqlSafe(sql))
        .bind(user.id)
        .bind(pager.offset)
        .bind(pager.limit);
    if let Some(section) = q.section {
        query = query.bind(section);
    }
    let topics = query.fetch_all(&state.pool).await?;

    if topics.is_empty() {
        return Err(AppError::NotFound);
    }
    let section_rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT DISTINCT s.id,s.name FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.userid=$1 AND NOT t.deleted AND NOT t.draft AND (t.moderate OR NOT s.moderate) ORDER BY s.id",
    ).bind(user.id).fetch_all(&state.pool).await?;
    let sections = section_rows
        .into_iter()
        .map(|(id, name)| UserSectionLink {
            id,
            name,
            selected: q.section == Some(id),
        })
        .collect();
    let base = format!("/people/{}/", urlencoding::encode(&user.nick));
    let query_prefix = q.section.map(|id| format!("section={id}"));
    let page_url = |new_offset: i64| {
        let mut params = Vec::new();
        if let Some(section) = &query_prefix {
            params.push(section.clone());
        }
        if new_offset > 0 {
            params.push(format!("offset={new_offset}"));
        }
        if params.is_empty() {
            base.clone()
        } else {
            format!("{base}?{}", params.join("&"))
        }
    };
    let prev_link = (pager.offset > 0).then(|| page_url((pager.offset - pager.limit).max(0)));
    let next_link = (topics.len() as i64 == pager.limit && pager.offset < 200)
        .then(|| page_url(pager.offset + pager.limit));
    let topics =
        crate::routes::topics::prepare_news_topics(&state, topics, q.section.is_none()).await?;
    Ok(Html(
        UserTopicsTemplate {
            title: format!("Сообщения {}", user.nick),
            nick: user.nick,
            topics,
            sections,
            all_selected: q.section.is_none(),
            prev_link,
            next_link,
        }
        .render()?,
    )
    .into_response())
}

pub async fn profile_full(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    RawQuery(optRawQuery): RawQuery,
    Query(q): Query<PagerQuery>,
    current: CurrentUser,
    stJar: CookieJar,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    if bHasRequestParameter(optRawQuery.as_deref(), "year-stats") {
        let stProfile = get_user_profile(&state, &nick).await?;
        if stProfile.blocked
            && !current
                .0
                .as_ref()
                .map(|stUser| stUser.canmod)
                .unwrap_or(false)
        {
            return Err(AppError::Forbidden);
        }

        let stTimezone = stRequestTimezone(&stJar);
        let cRepository = crate::infra::opensearch::CUserStatisticsOpenSearchRepository::new(
            state.config.opensearch_url.clone(),
            state.http.clone(),
        );
        let cService =
            crate::application::user::statistics::CUserStatisticsService::new(cRepository);
        let sTimezone = stTimezone.to_string();
        let mapStats = cService.mapYearStats(&stProfile.nick, &sTimezone).await?;
        return Ok(Json(mapStats).into_response());
    }

    if bHasRequestParameter(optRawQuery.as_deref(), "reset-password") {
        let _stModerator = current
            .0
            .as_ref()
            .filter(|stUser| stUser.canmod)
            .ok_or(AppError::Forbidden)?;
        let stProfile = get_user_profile(&state, &nick).await?;
        let sProfileLink = format!("/people/{}/profile", urlencoding::encode(&stProfile.nick));
        return Ok(Html(
            StResetPasswordConfirmationTemplate {
                iUserId: stProfile.id,
                sNick: stProfile.nick,
                sProfileLink,
                sCsrfToken: csrf_token,
            }
            .render()?,
        )
        .into_response());
    }
    let stTimezone = stRequestTimezone(&stJar);
    Ok(
        render_profile(state, nick, q, current, stTimezone, csrf_token)
            .await?
            .into_response(),
    )
}

fn bHasRequestParameter(optRawQuery: Option<&str>, sName: &str) -> bool {
    optRawQuery
        .and_then(|sRawQuery| serde_urlencoded::from_str::<HashMap<String, String>>(sRawQuery).ok())
        .is_some_and(|mapQuery| mapQuery.contains_key(sName))
}

fn sFuzzyDate(
    dtValue: chrono::DateTime<chrono::Utc>,
    stTimezone: chrono_tz::Tz,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> String {
    let stElapsed = dtNow - dtValue;
    if stElapsed < chrono::Duration::days(3) {
        "недавно".to_owned()
    } else if stElapsed < chrono::Duration::days(365) {
        dtValue
            .with_timezone(&stTimezone)
            .format("%d.%m.%y")
            .to_string()
    } else {
        dtValue.with_timezone(&stTimezone).format("%Y").to_string()
    }
}

async fn render_profile(
    state: AppState,
    nick: String,
    _q: PagerQuery,
    current: CurrentUser,
    stTimezone: chrono_tz::Tz,
    csrf_token: String,
) -> Result<Html<String>> {
    let profile = get_user_profile(&state, &nick).await?;
    let target_summary = get_user(&state, &nick).await?;
    if profile.blocked && current.0.is_none() {
        return Err(AppError::Forbidden);
    }
    if !profile.activated && !current.0.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let stats = user_stats(&state, profile.id).await?;
    let favorite_tags = user_tags(&state, profile.id, true).await?;
    let ignore_tags = user_tags(&state, profile.id, false).await?;
    let is_owner = current
        .0
        .as_ref()
        .map(|u| u.id == profile.id)
        .unwrap_or(false);
    let is_moderator = current.0.as_ref().map(|u| u.canmod).unwrap_or(false);
    let can_view_private = is_owner || is_moderator;
    let show_url = current.0.is_some() || profile.max_score >= 50;
    let show_userinfo = show_url;
    let remark = match current.0.as_ref().filter(|viewer| viewer.id != profile.id) {
        Some(viewer) => {
            sqlx::query_scalar(
                "SELECT remark_text FROM user_remarks WHERE user_id=$1 AND ref_user_id=$2",
            )
            .bind(viewer.id)
            .bind(profile.id)
            .fetch_optional(&state.pool)
            .await?
        }
        None => None,
    };
    let ignored = match current.0.as_ref().filter(|viewer| viewer.id != profile.id) {
        Some(viewer) => {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ignore_list WHERE userid=$1 AND ignored=$2)",
            )
            .bind(viewer.id)
            .bind(profile.id)
            .fetch_one(&state.pool)
            .await?
        }
        None => false,
    };
    let can_ignore = current
        .0
        .as_ref()
        .is_some_and(|viewer| viewer.id != profile.id && !profile.canmod);
    let can_remark = current
        .0
        .as_ref()
        .is_some_and(|viewer| viewer.id != profile.id);
    let has_remarks = if is_owner {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM user_remarks WHERE user_id=$1)")
            .bind(profile.id)
            .fetch_one(&state.pool)
            .await?
    } else {
        false
    };
    let can_load_userpic = if is_owner {
        crate::routes::legacy::bCanLoadUserpic(&state, &target_summary).await?
    } else {
        false
    };
    let (watch_present, fav_present) = if profile.anonymous {
        (false, false)
    } else {
        sqlx::query_as(
            r#"SELECT EXISTS(SELECT 1 FROM memories WHERE userid=$1 AND watch),
                      EXISTS(SELECT 1 FROM memories WHERE userid=$1 AND NOT watch)"#,
        )
        .bind(profile.id)
        .fetch_one(&state.pool)
        .await?
    };
    let drafts_count = if can_view_private {
        count_drafts(&state, profile.id).await.unwrap_or(0)
    } else {
        0
    };
    let bViewerFrozen = match current.0.as_ref() {
        Some(stViewer) => sqlx::query_scalar::<_, bool>(
            "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
        )
        .bind(stViewer.id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or(false),
        None => false,
    };
    let bViewerSlowMode = match current.0.as_ref() {
        Some(stViewer) => {
            crate::routes::topics::b_user_slow_mode_restricted(&state, stViewer).await?
        }
        None => false,
    };
    let bShowFuzzyLastLogin = !is_owner
        && current.0.as_ref().is_none_or(|stViewer| {
            stViewer.score.unwrap_or(0) < 100 || bViewerFrozen || bViewerSlowMode
        });
    let lastlogin_fuzzy = profile
        .lastlogin
        .filter(|_| bShowFuzzyLastLogin)
        .map(|dtLastLogin| sFuzzyDate(dtLastLogin, stTimezone, chrono::Utc::now()));
    // Was rendered with Askama's `|safe` straight from the raw DB column -
    // stored XSS via the "about me" field (POST /people/{nick}/edit). Route
    // it through the same sanitizing markup pipeline as comments/topics.
    let is_markdown = profile
        .userinfo_markup
        .as_deref()
        .map(|m| m.to_uppercase().contains("MARKDOWN"))
        .unwrap_or(false);
    let userinfo_html = profile
        .userinfo
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|text| markup::render_message(text, Some(!is_markdown)));

    // Moderation info: matches WhoisController's banInfo/isFrozen/
    // blockable/freezable/otherUsers/userlog fields, which the previous
    // implementation didn't surface at all - a moderator had no way to see
    // ban/freeze history or other accounts sharing an email from the
    // profile page itself.
    let ban_info = if profile.blocked {
        sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, String, String)>(
            r#"SELECT b.bandate, b.reason, u.nick FROM ban_info b JOIN users u ON u.id=b.ban_by WHERE b.userid=$1"#,
        )
        .bind(profile.id)
        .fetch_optional(&state.pool)
        .await?
        .map(|(bandate, reason, moderator_nick)| BanInfo {
            bandate,
            reason,
            moderator_nick,
        })
    } else {
        None
    };

    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(profile.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let is_frozen = frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false);
    let (frozen_within_three_days, recent_score_loss): (bool, i64) = sqlx::query_as(
        r#"SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP-interval '3 days',false),
                  COALESCE(abs((SELECT sum(di.bonus)::bigint FROM del_info di
                    WHERE di.deldate>CURRENT_TIMESTAMP-interval '3 days'
                      AND di.msgid IN (
                        SELECT c.id FROM comments c WHERE c.userid=$1
                        UNION ALL SELECT t.id FROM topics t WHERE t.userid=$1))),0)
             FROM users WHERE id=$1"#,
    )
    .bind(profile.id)
    .fetch_one(&state.pool)
    .await?;
    let slow_mode_reason = if profile.anonymous || profile.blocked || is_frozen {
        None
    } else if profile.score < 35 {
        Some("большое число нарушений правил, score < 35")
    } else if frozen_within_three_days {
        Some("заморозка закончилась менее трех дней назад")
    } else if recent_score_loss >= 30 {
        Some("превышен лимит нарушений правил за последние 3 дня")
    } else {
        None
    };
    let slow_mode = slow_mode_reason.is_some();
    let (freezer_nick, freezing_reason) = if is_frozen && current.0.is_some() && !bViewerFrozen {
        sqlx::query_as::<_, (Option<String>, Option<String>)>(
            r#"SELECT freezer.nick, target.freezing_reason
                 FROM users target LEFT JOIN users freezer ON freezer.id=target.frozen_by
                WHERE target.id=$1"#,
        )
        .bind(profile.id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or((None, None))
    } else {
        (None, None)
    };
    let url_nofollow = profile.score < 100 || profile.blocked || !profile.activated || is_frozen;
    let long_freeze_durations = frozen_until
        .and_then(|dtUntil| dtUntil.checked_add_months(chrono::Months::new(24)))
        .is_some_and(|dtTwoYearsAfterFreeze| dtTwoYearsAfterFreeze > chrono::Utc::now());
    let frozen_until = is_frozen.then(|| frozen_until.expect("is_frozen requires a timestamp"));

    // UserService.isBlockable/isFreezable: reuse the exact same rules
    // enforced server-side in usermod.jsp so the profile page never shows
    // a button that would then 403.
    let blockable = current
        .0
        .as_ref()
        .map(|u| !profile.anonymous && u.canmod && (!profile.canmod || u.candel))
        .unwrap_or(false);
    let freezable = current
        .0
        .as_ref()
        .map(|u| u.canmod && !profile.canmod)
        .unwrap_or(false);

    let other_accounts = if is_moderator {
        match profile.email.as_deref().filter(|e| !e.is_empty()) {
            Some(email) => sqlx::query_scalar(
                "SELECT nick FROM users WHERE lower(email)=lower($1) AND id<>$2 ORDER BY nick",
            )
            .bind(email)
            .bind(profile.id)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default(),
            None => vec![],
        }
    } else {
        vec![]
    };

    let user_log = if is_owner || is_moderator {
        sqlx::query_as::<_, (String, chrono::DateTime<chrono::Utc>, String)>(
            r#"SELECT l.action::text, l.action_date, u.nick
               FROM user_log l JOIN users u ON u.id=l.action_userid
               WHERE l.userid=$1 ORDER BY l.action_date DESC LIMIT 20"#,
        )
        .bind(profile.id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(action, date, actor_nick)| UserLogEntry {
            action,
            action_date: date,
            actor_nick,
        })
        .collect()
    } else {
        vec![]
    };

    // UserService.getUserpic: avatar fallback style is the *viewer's*
    // profile setting, not the target's.
    let viewer_avatar_mode = match &current.0 {
        Some(viewer) => {
            let settings_text: Option<String> =
                sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                    .bind(viewer.id)
                    .fetch_optional(&state.pool)
                    .await?;
            crate::profile::ProfileSettings::from_hstore_text(settings_text).avatar
        }
        None => crate::profile::DEFAULT_AVATAR.to_string(),
    };
    let userpic_url = crate::profile::userpic_url(
        &viewer_avatar_mode,
        true,
        profile.id == crate::routes::comments::ANONYMOUS_USER_ID,
        profile.photo.as_deref(),
        profile.email.as_deref(),
    )
    .unwrap_or_else(|| crate::profile::DISABLED_USERPIC.to_string());

    // UserService.getAllInvitedUsers / WhoisController "invitedUsers":
    // shown to everyone, not gated to owner/moderator, matching the original.
    let invited_users: Vec<String> = if can_view_private {
        sqlx::query_scalar(
            r#"SELECT u.nick FROM user_invites i JOIN users u ON u.id=i.invited_user
               WHERE i.owner=$1 AND i.invited_user IS NOT NULL ORDER BY i.issue_date"#,
        )
        .bind(profile.id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
    } else {
        Vec::new()
    };

    let year_stats_url = format!(
        "/people/{}/profile?year-stats",
        urlencoding::encode(&profile.nick)
    );
    let year_stats_user = profile.nick.clone();

    Ok(Html(
        UserTemplate {
            profile,
            stats,
            favorite_tags,
            ignore_tags,
            drafts_count,
            is_owner,
            is_moderator,
            can_view_private,
            userinfo_html,
            ban_info,
            frozen_until,
            is_frozen,
            long_freeze_durations,
            blockable,
            freezable,
            other_accounts,
            user_log,
            invited_users,
            lastlogin_fuzzy,
            show_url,
            show_userinfo,
            url_nofollow,
            remark,
            can_remark,
            ignored,
            can_ignore,
            has_remarks,
            can_load_userpic,
            watch_present,
            fav_present,
            slow_mode,
            slow_mode_reason: slow_mode_reason.unwrap_or_default().to_string(),
            freezer_nick,
            freezing_reason,
            userpic_url,
            year_stats_url,
            year_stats_user,
            csrf_token,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct WhoisQuery {
    nick: String,
}

pub async fn legacy_whois(Query(q): Query<WhoisQuery>) -> Response {
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/people/{}/profile", urlencoding::encode(&q.nick)),
        )],
    )
        .into_response()
}

const REACTIONS_ITEMS_PER_PAGE: i64 = 50;
const REACTIONS_MAX_OFFSET: i64 = 10000;

const SECTION_PREFIX_CASE: &str = "CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END";

#[derive(Debug, sqlx::FromRow)]
struct ReactionViewRow {
    topic_id: i32,
    comment_id: Option<i32>,
    set_date: chrono::DateTime<chrono::Utc>,
    reaction: String,
    title: String,
    target_user: i32,
    section_prefix: String,
    group_urlname: String,
}

impl ReactionViewRow {
    fn link(&self) -> String {
        let anchor = self
            .comment_id
            .map(|id| format!("?cid={id}"))
            .unwrap_or_default();
        format!(
            "/{}/{}/{}{anchor}",
            self.section_prefix, self.group_urlname, self.topic_id
        )
    }
}

#[derive(Deserialize)]
pub struct ReactionsQuery {
    pub offset: Option<i64>,
}

/// UserReactionsController.reactions ("мои реакции" mode, mode == null).
pub async fn reactions(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<ReactionsQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    reactions_view(&state, nick, None, q, current).await
}

/// UserReactionsController.reactions with `{mode}` path segment - only "to"
/// ("реакции на меня") is recognised, matching the Java `BadParameterException`
/// for anything else.
pub async fn reactions_mode(
    State(state): State<AppState>,
    Path((nick, mode)): Path<(String, String)>,
    Query(q): Query<ReactionsQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    reactions_view(&state, nick, Some(mode), q, current).await
}

async fn reactions_view(
    state: &AppState,
    nick: String,
    mode: Option<String>,
    q: ReactionsQuery,
    current: CurrentUser,
) -> Result<Html<String>> {
    let user = get_user(state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let current_user = current.0.expect("checked by ensure_self_or_moderator");

    let offset = q.offset.unwrap_or(0);
    if offset > REACTIONS_MAX_OFFSET {
        return Err(AppError::BadRequest("offset too big".into()));
    }

    let mode_to = match mode.as_deref() {
        None => false,
        Some(m) if m.eq_ignore_ascii_case("to") => true,
        Some(_) => return Err(AppError::BadRequest("incorrect mode".into())),
    };

    // Java's ReactionDao.getReactionsView `includeDeleted` flag - only
    // moderators see reactions on deleted topics/comments.
    let show_deleted = current_user.canmod;
    let limit = REACTIONS_ITEMS_PER_PAGE + 1;

    let items: Vec<ReactionViewRow> = if mode_to {
        let not_deleted_topic = if show_deleted {
            ""
        } else {
            "AND NOT t.deleted"
        };
        let not_deleted_comment = if show_deleted {
            ""
        } else {
            "AND NOT c.deleted"
        };
        let sql = format!(
            r#"SELECT r.topic_id, r.comment_id, r.set_date, r.reaction, t.title,
                      r.origin_user AS target_user,
                      {SECTION_PREFIX_CASE} AS section_prefix,
                      g.urlname AS group_urlname
               FROM reactions_log r
               JOIN topics t ON r.topic_id = t.id {not_deleted_topic}
               JOIN groups g ON t.groupid = g.id
               JOIN sections s ON s.id = g.section
               WHERE r.comment_id IS NULL AND t.userid = $1
               UNION ALL
               SELECT r.topic_id, r.comment_id, r.set_date, r.reaction, t.title,
                      r.origin_user AS target_user,
                      {SECTION_PREFIX_CASE} AS section_prefix,
                      g.urlname AS group_urlname
               FROM reactions_log r
               JOIN topics t ON r.topic_id = t.id {not_deleted_topic}
               JOIN comments c ON c.id = r.comment_id
               JOIN groups g ON t.groupid = g.id
               JOIN sections s ON s.id = g.section
               WHERE c.userid = $1 {not_deleted_comment}
               ORDER BY set_date DESC OFFSET $2 LIMIT $3"#
        );
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.id)
            .bind(offset)
            .bind(limit)
            .fetch_all(&state.pool)
            .await?
    } else {
        let not_deleted = if show_deleted {
            ""
        } else {
            "AND NOT t.deleted AND c.deleted IS NOT TRUE"
        };
        let sql = format!(
            r#"SELECT r.topic_id, r.comment_id, r.set_date, r.reaction, t.title,
                      COALESCE(c.userid, t.userid) AS target_user,
                      {SECTION_PREFIX_CASE} AS section_prefix,
                      g.urlname AS group_urlname
               FROM reactions_log r
               JOIN topics t ON r.topic_id = t.id
               JOIN groups g ON t.groupid = g.id
               JOIN sections s ON s.id = g.section
               LEFT JOIN comments c ON r.comment_id = c.id
               WHERE r.origin_user = $1 {not_deleted}
               ORDER BY r.set_date DESC OFFSET $2 LIMIT $3"#
        );
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.id)
            .bind(offset)
            .bind(limit)
            .fetch_all(&state.pool)
            .await?
    };

    let has_more = items.len() as i64 > REACTIONS_ITEMS_PER_PAGE;
    let items: Vec<ReactionViewRow> = items
        .into_iter()
        .take(REACTIONS_ITEMS_PER_PAGE as usize)
        .collect();

    let target_ids: Vec<i32> = items.iter().map(|r| r.target_user).collect();
    let target_nicks: HashMap<i32, String> = if target_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (i32, String)>("SELECT id, nick FROM users WHERE id = ANY($1)")
            .bind(&target_ids)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .collect()
    };

    let message_ids: Vec<i64> = items
        .iter()
        .map(|r| (r.comment_id.unwrap_or(r.topic_id)) as i64)
        .collect();
    let previews: HashMap<i32, String> = if message_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (i64, String)>("SELECT id, message FROM msgbase WHERE id = ANY($1)")
            .bind(&message_ids)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(|(id, message)| {
                let id = id as i32;
                let plain = markup::plain_text_for_index(&message);
                let trimmed = if plain.chars().count() > 250 {
                    format!("{}...", plain.chars().take(250).collect::<String>().trim())
                } else {
                    plain
                };
                (id, trimmed)
            })
            .collect()
    };

    let base_url = format!("/people/{}/reactions", user.nick);
    let to_url = format!("{base_url}/to");
    let url = if mode_to {
        to_url.clone()
    } else {
        base_url.clone()
    };
    let me_title = if user.id == current_user.id {
        "мои реакции".to_string()
    } else {
        format!("реакции {}", user.nick)
    };
    let reactions_title = if user.id == current_user.id {
        "на мои сообщения".to_string()
    } else {
        format!("реакции на {}", user.nick)
    };

    let mut html = String::from("<h1>Реакции</h1><div class=\"reactions-view\"><p>");
    if mode_to {
        html.push_str(&format!(
            "<a class=\"btn btn-default\" href=\"{}\">{}</a> ",
            html_escape::encode_text(&base_url),
            html_escape::encode_text(&me_title)
        ));
        html.push_str(&format!(
            "<a class=\"btn btn-selected\" href=\"{}\">{}</a>",
            html_escape::encode_text(&to_url),
            html_escape::encode_text(&reactions_title)
        ));
    } else {
        html.push_str(&format!(
            "<a class=\"btn btn-selected\" href=\"{}\">{}</a> ",
            html_escape::encode_text(&base_url),
            html_escape::encode_text(&me_title)
        ));
        html.push_str(&format!(
            "<a class=\"btn btn-default\" href=\"{}\">{}</a>",
            html_escape::encode_text(&to_url),
            html_escape::encode_text(&reactions_title)
        ));
    }
    html.push_str("</p>");

    for item in &items {
        let target_nick = target_nicks
            .get(&item.target_user)
            .map(|s| s.as_str())
            .unwrap_or("");
        let preview = previews
            .get(&item.comment_id.unwrap_or(item.topic_id))
            .map(|s| s.as_str())
            .unwrap_or("");
        let sDate = crate::request_timezone::sTimeTag("compact-interval", item.set_date);
        html.push_str(&format!(
            r#"<a class="reactions-view-item" href="{}">
                 <div class="reactions-view-reaction"><p>{}</p></div>
                 <div class="reactions-view-title"><p>{}{}</p></div>
                 <div class="reactions-view-date"><p>{}</p></div>
                 <div class="reactions-view-preview"><div class="text-preview-box"><div class="text-preview">{}: {}</div></div></div>
               </a>"#,
            html_escape::encode_text(&item.link()),
            html_escape::encode_text(&item.reaction),
            if item.comment_id.is_some() { "<i class=\"icon-comment\"></i> " } else { "" },
            html_escape::encode_text(&item.title),
            sDate,
            html_escape::encode_text(target_nick),
            html_escape::encode_text(preview),
        ));
    }
    html.push_str("</div>");

    html.push_str("<table class=\"nav\"><tr>");
    if offset >= REACTIONS_ITEMS_PER_PAGE {
        let prev_offset = offset - REACTIONS_ITEMS_PER_PAGE;
        let prev_url = if prev_offset == 0 {
            url.clone()
        } else {
            format!("{url}?offset={prev_offset}")
        };
        html.push_str(&format!(
            r#"<td width="35%" align="left"><a href="{}">← предыдущие</a></td>"#,
            html_escape::encode_text(&prev_url)
        ));
    }
    // Java permits offset == MaxOffset and therefore emits the final link
    // from 9950 to 10000 when a full look-ahead row exists.
    if has_more && offset < REACTIONS_MAX_OFFSET {
        html.push_str(&format!(
            r#"<td align="right" width="35%"><a href="{url}?offset={}">следующие →</a></td>"#,
            offset + REACTIONS_ITEMS_PER_PAGE,
            url = html_escape::encode_text(&url)
        ));
    }
    html.push_str("</tr></table>");

    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct StRemarksQuery {
    pub offset: Option<i64>,
    pub sort: Option<i32>,
}

#[derive(Debug)]
struct StRemarkListRow {
    sNick: String,
    sText: String,
}

#[derive(Template)]
#[template(path = "remarks.html")]
struct StRemarksTemplate {
    sNick: String,
    vecRemarks: Vec<StRemarkListRow>,
    iOffset: i64,
    iLimit: i64,
    iSort: i32,
    bHasMore: bool,
}

pub async fn remarks(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(stQuery): Query<StRemarksQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    // Java's ShowRemarkController only ever shows the logged-in user's OWN
    // remarks about other people (keyed by user_id = viewer), never other
    // people's remarks about the profile being viewed - it is a private
    // notebook, not a public annotation feed. `nick` must equal the viewer.
    let Some(me) = current.0 else {
        return Err(AppError::Forbidden);
    };
    if !me.nick.eq_ignore_ascii_case(&nick) {
        return Err(AppError::Forbidden);
    }
    let iOffset = stQuery.offset.unwrap_or(0);
    let iSort = stQuery.sort.unwrap_or(0);
    if !matches!(iSort, 0 | 1) {
        return Err(AppError::BadRequest("Wrong sort".into()));
    }
    let iCount: i64 = sqlx::query_scalar("SELECT count(*) FROM user_remarks WHERE user_id=$1")
        .bind(me.id)
        .fetch_one(&state.pool)
        .await?;
    if iCount > 0 && (iOffset < 0 || iOffset >= iCount) {
        return Err(AppError::BadRequest("Wrong offset".into()));
    }
    let iLimit = crate::routes::topics::messages_per_page(&state, &Some(me.clone())).await;
    let sOrder = if iSort == 1 {
        "r.remark_text ASC"
    } else {
        "u.nick ASC"
    };
    let sSql = format!(
        "SELECT u.nick, r.remark_text FROM user_remarks r JOIN users u ON u.id=r.ref_user_id WHERE r.user_id=$1 ORDER BY {sOrder} LIMIT $2 OFFSET $3"
    );
    let vecRemarks = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(sSql))
        .bind(me.id)
        .bind(iLimit)
        .bind(iOffset)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(sNick, sText)| StRemarkListRow { sNick, sText })
        .collect();
    Ok(Html(
        StRemarksTemplate {
            sNick: me.nick,
            vecRemarks,
            iOffset,
            iLimit,
            iSort,
            bHasMore: iCount > iOffset + iLimit,
        }
        .render()?,
    ))
}

pub async fn get_user(state: &AppState, nick: &str) -> Result<UserSummary> {
    sqlx::query_as::<_, UserSummary>(
        "SELECT id,nick,name,score,max_score,photo,town,regdate,canmod,COALESCE(candel,false) AS candel,COALESCE(corrector,false) AS corrector,blocked,userinfo FROM users WHERE lower(nick)=lower($1)",
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn get_user_profile(state: &AppState, nick: &str) -> Result<UserProfileData> {
    sqlx::query_as::<_, UserProfileData>(
        r#"SELECT id, nick, name,
                  COALESCE(score,0) AS score,
                  COALESCE(max_score,0) AS max_score,
                  photo, town, userinfo, url, email,
                  COALESCE(canmod,false) AS canmod,
                  COALESCE(candel,false) AS candel,
                  COALESCE(passwd,'')='' AS anonymous,
                  COALESCE(corrector,false) AS corrector,
                  COALESCE(blocked,false) AS blocked,
                  COALESCE(activated,true) AS activated,
                  regdate, lastlogin,
                  userinfo_markup::text AS userinfo_markup
           FROM users WHERE lower(nick)=lower($1)"#,
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn user_stats(state: &AppState, user_id: i32) -> Result<UserStats> {
    let (topic_count, first_topic, last_topic): (
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT count(*)::bigint, min(postdate), max(postdate) FROM topics WHERE userid=$1 AND NOT COALESCE(deleted,false) AND NOT COALESCE(draft,false)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    let (comment_count, first_comment, last_comment): (
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT count(*)::bigint, min(postdate), max(postdate) FROM comments WHERE userid=$1 AND NOT COALESCE(deleted,false)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(UserStats {
        topic_count,
        comment_count,
        first_topic,
        last_topic,
        first_comment,
        last_comment,
    })
}

pub(crate) async fn user_tags(
    state: &AppState,
    user_id: i32,
    favorite: bool,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT tv.value FROM user_tags ut JOIN tags_values tv ON tv.id=ut.tag_id WHERE ut.user_id=$1 AND ut.is_favorite=$2 ORDER BY tv.value",
    )
    .bind(user_id)
    .bind(favorite)
    .fetch_all(&state.pool)
    .await?)
}

async fn count_drafts(state: &AppState, user_id: i32) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint FROM topics WHERE userid=$1 AND COALESCE(draft,false)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?)
}

pub async fn deleted_topics(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<PagerQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    let stTarget = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &stTarget)?;
    render_user_topic_list(state, nick, q, "Удалённые темы", "t.deleted").await
}

pub async fn drafts(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<PagerQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    let stTarget = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &stTarget)?;
    render_user_topic_list(state, nick, q, "Черновики", "t.draft").await
}

pub async fn favs(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<PagerQuery>,
) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, au.id AS author_id, au.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM memories mem
           JOIN topics t ON t.id=mem.topic
           JOIN users au ON au.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE mem.userid=$1 AND NOT mem.watch AND NOT t.deleted
           ORDER BY mem.add_date DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user.id)
    .bind(pager.offset)
    .bind(pager.limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(simple_topic_list(
        &format!("Избранное {}", user.nick),
        &topics,
    )))
}

pub async fn tracked(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(q): Query<PagerQuery>,
    current: CurrentUser,
) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, au.id AS author_id, au.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM memories mem
           JOIN topics t ON t.id=mem.topic
           JOIN users au ON au.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE mem.userid=$1 AND mem.watch AND NOT t.deleted
           ORDER BY mem.add_date DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user.id)
    .bind(pager.offset)
    .bind(pager.limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(simple_topic_list(
        &format!("Отслеживаемое {}", user.nick),
        &topics,
    )))
}

async fn render_user_topic_list(
    state: AppState,
    nick: String,
    q: PagerQuery,
    title: &str,
    predicate: &'static str,
) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE u.id=$1 AND {predicate}
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#
    );
    let topics = sqlx::query_as::<_, TopicSummary>(sqlx::AssertSqlSafe(sql))
        .bind(user.id)
        .bind(pager.offset)
        .bind(pager.limit)
        .fetch_all(&state.pool)
        .await?;
    Ok(Html(simple_topic_list(
        &format!("{title} {}", user.nick),
        &topics,
    )))
}

fn simple_topic_list(title: &str, topics: &[TopicSummary]) -> String {
    let mut html = format!(
        "<h1>{}</h1><div class=\"topic-list\">",
        html_escape::encode_text(title)
    );
    for t in topics {
        html.push_str(&format!("<article class=\"topic-card\"><h3><a href=\"{}\">{}</a></h3><div class=\"meta\">{} · {} комментариев</div></article>", t.topic_url(), html_escape::encode_text(&t.title), crate::request_timezone::sTimeTag("default", t.postdate), t.comments));
    }
    html.push_str("</div>");
    html
}

fn optEditProfileInfoRestriction(
    bFrozen: bool,
    bRecentResetInfo: bool,
    bRecentResetUrl: bool,
    bRecentResetTown: bool,
) -> Option<&'static str> {
    if bFrozen {
        Some("установлен режим только для чтения")
    } else if bRecentResetInfo {
        Some("текст профиля был сброшен модератором менее 24 часов назад")
    } else if bRecentResetUrl {
        Some("url был сброшен модератором менее 24 часов назад")
    } else if bRecentResetTown {
        Some("поле города было сброшено модератором менее 24 часов назад")
    } else {
        None
    }
}

async fn optEditProfileInfoRestrictionForUser(
    state: &AppState,
    user_id: i32,
) -> Result<Option<&'static str>> {
    let row: (bool, bool, bool, bool) = sqlx::query_as(
        r#"SELECT COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false),
                  EXISTS(SELECT 1 FROM user_log l WHERE l.userid=u.id
                    AND l.action::text='reset_info'
                    AND l.action_date>CURRENT_TIMESTAMP-interval '1 day'
                    AND l.userid<>l.action_userid),
                  EXISTS(SELECT 1 FROM user_log l WHERE l.userid=u.id
                    AND l.action::text='reset_url'
                    AND l.action_date>CURRENT_TIMESTAMP-interval '1 day'
                    AND l.userid<>l.action_userid),
                  EXISTS(SELECT 1 FROM user_log l WHERE l.userid=u.id
                    AND l.action::text='reset_town'
                    AND l.action_date>CURRENT_TIMESTAMP-interval '1 day'
                    AND l.userid<>l.action_userid)
             FROM users u WHERE u.id=$1"#,
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(optEditProfileInfoRestriction(row.0, row.1, row.2, row.3))
}

fn bValidProfileUrl(value: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)^((((https?)|(ftp))://(([0-9\p{L}.-]+\.[0-9\p{L}]+)|(\d+\.\d+\.\d+\.\d+))(:[0-9]+)?(/[^ ]*)?)|(mailto:[a-z0-9_+-.]+@[0-9a-z.-]+\.[a-z]+)|(news:[a-z0-9.-]+)|(((www)|(ftp))\.(([0-9a-z.-]+\.[a-z]+(:[0-9]+)?(/[^ ]*)?)|([a-z]+(/[^ ]*)?))))$",
        )
        .expect("Java-compatible profile URL regex must compile")
    })
    .is_match(value)
}

fn optFixedProfileUrl(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !bValidProfileUrl(value) {
        return Err(AppError::BadRequest("Некорректный URL".into()));
    }
    let value = value.trim();
    if value.to_ascii_lowercase().starts_with("www.") {
        Ok(Some(format!("http://{value}")))
    } else if value.to_ascii_lowercase().starts_with("ftp.") {
        Ok(Some(format!("ftp://{value}")))
    } else {
        Ok(Some(value.to_string()))
    }
}

fn sMarkupIdFromForm(value: &str) -> &'static str {
    match value {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        _ => "BBCODE_TEX",
    }
}

#[derive(Deserialize)]
pub struct ProfileForm {
    pub name: Option<String>,
    pub town: Option<String>,
    pub url: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "info", alias = "userinfo")]
    pub info: Option<String>,
    #[serde(rename = "infoMarkup")]
    pub info_markup: Option<String>,
    pub password: Option<String>,
    pub password2: Option<String>,
    pub oldpass: Option<String>,
}

pub async fn edit_profile_form(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    ensure_self_service_actor(&current.0, &nick)?;
    let user = get_user(&state, &nick).await?;
    let profile = get_user_profile(&state, &nick).await?;
    ensure_self(&current.0, &user)?;
    let can_load_userpic = crate::routes::legacy::bCanLoadUserpic(&state, &user).await?;
    let opt_restriction = optEditProfileInfoRestrictionForUser(&state, user.id).await?;
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?;
    let settings = ProfileSettings::from_hstore_text(settings_text);
    let effective_markup = if profile
        .userinfo
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        settings.format_mode.clone()
    } else {
        crate::routes::topics::markup_form_view(
            profile.userinfo_markup.as_deref().unwrap_or("BBCODE_TEX"),
        )
        .0
    };
    let info_markup_title = crate::profile::FORMAT_MODES
        .iter()
        .find(|(value, _, _)| *value == effective_markup)
        .map(|(_, title, _)| (*title).to_string())
        .unwrap_or_else(|| crate::routes::topics::markup_form_view("BBCODE_TEX").1);
    let mut response = Html(
        EditProfileTemplate {
            user,
            profile,
            can_load_userpic,
            can_edit_info: opt_restriction.is_none(),
            can_edit_info_reason: opt_restriction.unwrap_or_default().to_string(),
            info_markup_form_id: effective_markup,
            info_markup_title,
            csrf_token,
        }
        .render()?,
    )
    .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate"
            .parse()
            .expect("static cache-control value"),
    );
    Ok(response)
}

pub async fn edit_profile(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    headers: HeaderMap,
    ConnectInfo(peer_address): ConnectInfo<SocketAddr>,
    Form(form): axum::Form<ProfileForm>,
) -> Result<impl axum::response::IntoResponse> {
    ensure_self_service_actor(&current.0, &nick)?;
    let user = get_user(&state, &nick).await?;
    // Java's EditProfileController is strictly self-service (no moderator
    // override) and requires the current password before touching anything.
    ensure_self(&current.0, &user)?;

    let oldpass = form.oldpass.as_deref().unwrap_or("");
    if oldpass.is_empty() {
        return Err(AppError::BadRequest(
            "Для изменения регистрации нужен ваш пароль".into(),
        ));
    }
    let current_hash: Option<String> = sqlx::query_scalar("SELECT passwd FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    if !current_hash
        .as_deref()
        .map(|hash| security::password::verify(oldpass, hash))
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest("Неверный пароль".into()));
    }

    let new_password = form.password.as_deref().filter(|s| !s.is_empty());
    let new_password_hash = if let Some(password) = new_password {
        if password.eq_ignore_ascii_case(&user.nick) {
            return Err(AppError::BadRequest(
                "пароль не может совпадать с логином".to_string(),
            ));
        }
        if form.password2.as_deref() != Some(password) {
            return Err(AppError::BadRequest(
                "введенные пароли не совпадают".to_string(),
            ));
        }
        if password.chars().count() < 10 {
            return Err(AppError::BadRequest(
                "слишком короткий пароль, минимальная длина: 10".to_string(),
            ));
        }
        Some(
            security::password::hash(password)
                .map_err(|e| AppError::BadRequest(format!("password hash error: {e}")))?,
        )
    } else {
        None
    };

    // Email changes are staged into new_email and only take effect once the
    // user follows the activation-code link (see legacy::activate_post),
    // matching Java's UserDao.setNewEmail / acceptNewEmail split - the
    // previous handler wrote straight to `email` with no confirmation at all.
    let profile = get_user_profile(&state, &nick).await?;
    let regdate = profile.regdate;
    let requested_email = form
        .email
        .as_deref()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty());
    let Some(requested_email) = requested_email else {
        return Err(AppError::BadRequest("Не указан e-mail".into()));
    };
    if requested_email.matches('@').count() != 1 || requested_email.chars().any(char::is_whitespace)
    {
        return Err(AppError::BadRequest("Некорректный e-mail".into()));
    }
    crate::routes::auth::validate_registration_email(&state, &requested_email).await?;
    let pending_email =
        (Some(requested_email.as_str()) != profile.email.as_deref()).then_some(requested_email);

    if let Some(ref new_email) = pending_email {
        let taken: Option<i32> = sqlx::query_scalar(
            r#"SELECT id FROM users
               WHERE normalize_email(email)=normalize_email($1) AND NOT blocked
               ORDER BY id DESC LIMIT 1"#,
        )
        .bind(new_email)
        .fetch_optional(&state.pool)
        .await?;
        if taken.is_some() {
            return Err(AppError::BadRequest("такой email уже используется".into()));
        }
    }

    let client_ip = crate::security::stClientIp(
        peer_address.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let ip_block: Option<(bool, bool)> = sqlx::query_as(
        r#"SELECT (ban_date IS NULL OR ban_date>CURRENT_TIMESTAMP),
                  COALESCE(allow_posting,false)
             FROM b_ips WHERE ip=$1::inet"#,
    )
    .bind(&client_ip)
    .fetch_optional(&state.pool)
    .await?;
    if ip_block.is_some_and(|(blocked, allow_posting)| blocked && !allow_posting) {
        return Err(AppError::BadRequest(
            "постинг с этого IP адреса заблокирован".into(),
        ));
    }

    let opt_restriction = optEditProfileInfoRestrictionForUser(&state, user.id).await?;
    let can_edit_info = opt_restriction.is_none();
    let fixed_url = optFixedProfileUrl(form.url.as_deref())?;
    let name = form
        .name
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| html_escape::encode_text(value).into_owned());
    let town = form
        .town
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| html_escape::encode_text(value).into_owned());
    if town
        .as_deref()
        .is_some_and(|value| value.chars().count() > 100)
    {
        return Err(AppError::BadRequest(
            "Слишком длинное название города (максимум 100 символов)".into(),
        ));
    }
    let info = form.info.filter(|value| !value.is_empty());
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?;
    let settings = ProfileSettings::from_hstore_text(settings_text);
    let requested_markup = form
        .info_markup
        .as_deref()
        .filter(|value| crate::profile::is_format_mode(value))
        .unwrap_or(&settings.format_mode);
    let info_markup = sMarkupIdFromForm(requested_markup);

    let mut tx = state.pool.begin().await?;
    if let Some(ref hash) = new_password_hash {
        sqlx::query("UPDATE users SET passwd=$2,lostpwd='epoch' WHERE id=$1")
            .bind(user.id)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        crate::audit::log_user_action_tx(
            &mut tx,
            user.id,
            user.id,
            "set_password",
            &[("ip", client_ip.as_str())],
        )
        .await?;
    }
    if let Some(ref new_email) = pending_email {
        sqlx::query("UPDATE users SET new_email=$2 WHERE id=$1")
            .bind(user.id)
            .bind(new_email)
            .execute(&mut *tx)
            .await?;
    }
    if can_edit_info {
        sqlx::query(
            r#"UPDATE users SET name=$2,town=$3,url=$4,userinfo=$5,
                       userinfo_markup=$6::markup_type WHERE id=$1"#,
        )
        .bind(user.id)
        .bind(&name)
        .bind(&town)
        .bind(&fixed_url)
        .bind(&info)
        .bind(info_markup)
        .execute(&mut *tx)
        .await?;
        let mut changed = Vec::new();
        if profile.name != name {
            changed.push(("name", name.as_deref().unwrap_or("")));
        }
        if profile.url != fixed_url {
            changed.push(("url", fixed_url.as_deref().unwrap_or("")));
        }
        if profile.town != town {
            changed.push(("town", town.as_deref().unwrap_or("")));
        }
        if profile.userinfo != info || profile.userinfo_markup.as_deref() != Some(info_markup) {
            changed.push(("info", info.as_deref().unwrap_or("")));
        }
        if !changed.is_empty() {
            crate::audit::log_user_action_tx(&mut tx, user.id, user.id, "set_info", &changed)
                .await?;
        }
    }
    tx.commit().await?;

    if let Some(ref new_email) = pending_email {
        let regdate = regdate.ok_or_else(|| {
            AppError::Anyhow(anyhow::anyhow!("user registration date is missing"))
        })?;
        crate::routes::auth::cEmailService(&state)
            .vSendRegistration(&user.nick, new_email, regdate.timestamp_millis(), false)
            .await?;
        Ok(Html(
            "<h1>Обновление регистрации прошло успешно</h1><p>Ожидайте письма с кодом активации смены email.</p>".to_string(),
        ).into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/people/{}/profile", urlencoding::encode(&user.nick)),
            )],
        )
            .into_response())
    }
}

pub async fn settings(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    ensure_self_service_actor(&current.0, &nick)?;
    let user = get_user(&state, &nick).await?;
    // Java's EditSettingsController is strictly self-service, no moderator override.
    ensure_self(&current.0, &user)?;
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?;
    let settings = ProfileSettings::from_hstore_text(settings_text);
    // edit-settings.jsp disables this below one green star unless a legacy
    // profile already has the option enabled.
    let hide_adsense_disabled = user.score.unwrap_or(0) < 100 && !settings.hide_adsense;
    Ok(Html(
        SettingsTemplate {
            themes: settings.theme_options(user.score.unwrap_or(0)),
            avatars: settings.avatar_options(),
            tracker_modes: settings.tracker_options(),
            format_modes: settings.format_options(user.score.unwrap_or(0)),
            topic_values: settings.topic_options(),
            message_values: settings.message_options(),
            user,
            settings,
            hide_adsense_disabled,
            csrf_token,
        }
        .render()?,
    )
    .into_response())
}

pub async fn save_settings(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    Form(form): axum::Form<HashMap<String, String>>,
) -> Result<Response> {
    ensure_self_service_actor(&current.0, &nick)?;
    let user = get_user(&state, &nick).await?;
    ensure_self(&current.0, &user)?;
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?;
    let current_settings = ProfileSettings::from_hstore_text(settings_text);
    let settings = current_settings
        .apply_form(&form)
        .map_err(AppError::BadRequest)?;
    let (keys, values) = settings.to_hstore_arrays();
    sqlx::query(
        "INSERT INTO user_settings(id,settings) VALUES($1,hstore($2::text[],$3::text[])) ON CONFLICT(id) DO UPDATE SET settings=EXCLUDED.settings",
    )
    .bind(user.id)
    .bind(keys)
    .bind(values)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/people/{}/profile", urlencoding::encode(&user.nick)),
        )],
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct RemarkForm {
    pub text: String,
}

pub async fn remark_form(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let target = get_user(&state, &nick).await?;
    let Some(me) = current.0 else {
        return Err(AppError::Forbidden);
    };
    if me.id == target.id {
        return Err(AppError::BadRequest(
            "Нельзя оставить заметку самому себе".into(),
        ));
    }
    let remark: Option<String> = sqlx::query_scalar(
        "SELECT remark_text FROM user_remarks WHERE user_id=$1 AND ref_user_id=$2",
    )
    .bind(me.id)
    .bind(target.id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Html(format!(
        r#"
<h1>Заметка о {}</h1>
<form method="post" action="/people/{}/remark">
<input type="hidden" name="csrf" value="{csrf_token}">
<textarea autofocus id="text" name="text" cols="60" rows="4" maxlength="255">{}</textarea>
<button type="submit">Сохранить</button>
</form>
"#,
        html_escape::encode_text(&target.nick),
        urlencoding::encode(&target.nick),
        html_escape::encode_text(remark.as_deref().unwrap_or(""))
    )))
}

pub async fn save_remark(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    Form(form): axum::Form<RemarkForm>,
) -> Result<Response> {
    let target = get_user(&state, &nick).await?;
    let Some(me) = current.0 else {
        return Err(AppError::Forbidden);
    };
    if me.id == target.id {
        return Err(AppError::BadRequest(
            "Нельзя оставить заметку самому себе".into(),
        ));
    }
    let text: String = form.text.chars().take(255).collect();
    if text.is_empty() {
        sqlx::query("DELETE FROM user_remarks WHERE user_id=$1 AND ref_user_id=$2")
            .bind(me.id)
            .bind(target.id)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO user_remarks(user_id,ref_user_id,remark_text) VALUES($1,$2,$3) ON CONFLICT(user_id,ref_user_id) DO UPDATE SET remark_text=EXCLUDED.remark_text",
        )
        .bind(me.id).bind(target.id).bind(text).execute(&state.pool).await?;
    }
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/people/{}/profile", urlencoding::encode(&target.nick)),
        )],
    )
        .into_response())
}

/// Java's `/people/{nick}/profile/wipe` is GET/HEAD-only and purely a
/// moderator confirmation view (`UserModificationController.wipe`) - the
/// actual destructive action lives behind a separate POST to
/// `/usermod.jsp?action=block-n-delete-comments`. The previous Rust handler
/// collapsed both into one plain GET that any logged-in user (self included)
/// could trigger with no confirmation step - fixed to match: moderator-only,
/// no side effects, renders a form that posts to the real action endpoint.
pub async fn profile_wipe(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    current: CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    let moderator = current
        .0
        .as_ref()
        .filter(|u| u.canmod)
        .ok_or(AppError::Forbidden)?;
    let user = get_user_profile(&state, &nick).await?;
    if user.anonymous || !moderator.canmod || (user.canmod && !moderator.candel) {
        return Err(AppError::Forbidden);
    }
    if user.blocked {
        return Ok(crate::routes::admin::stUserModErrorResponse(
            "Пользователь уже блокирован".to_owned(),
        ));
    }
    let comment_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM comments WHERE userid=$1 AND NOT deleted")
            .bind(user.id)
            .fetch_one(&state.pool)
            .await?;
    Ok(Html(format!(
        r#"
<h1>Блокировка с удалением сообщений {nick}</h1>
<p>Будет удалено {comment_count} комментариев вместе с ответами, а так же все темы пользователя.</p>
<p><strong>Внимание!</strong> простого способа востановить комментарии после удаления нет.</p>
<form method="post" action="/usermod.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="action" value="block-n-delete-comments">
  <input type="hidden" name="id" value="{id}">
  <label>Причина <input type="text" name="reason" size="60" autofocus></label>
  <button type="submit" class="btn btn-danger">Блокировать с удалением сообщений</button>
</form>
"#,
        nick = html_escape::encode_text(&user.nick),
        id = user.id
    ))
    .into_response())
}

fn ensure_self_or_moderator(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else {
        return Err(AppError::Forbidden);
    };
    if current.id == target.id || current.canmod {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Java's `AuthorizedOnly` wraps both profile self-service controllers and
/// raises `AccessViolationException` for anonymous visitors as well as for a
/// different account. Check this before any target-user database lookup.
fn ensure_self_service_actor(optCurrent: &Option<UserSummary>, sTargetNick: &str) -> Result<()> {
    let Some(stCurrent) = optCurrent else {
        return Err(AppError::Forbidden);
    };
    if stCurrent.nick != sTargetNick {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

/// Strictly self-service, no moderator override - matches Java controllers
/// (e.g. EditProfileController) that reject even moderators editing someone
/// else's registration through this path.
fn ensure_self(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else {
        return Err(AppError::Forbidden);
    };
    if current.id == target.id {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bHasRequestParameter, edit_profile_form, ensure_self_service_actor,
        optEditProfileInfoRestriction, optFixedProfileUrl, sMarkupIdFromForm, settings,
    };
    use crate::{config::StConfig, error::AppError, models::UserSummary, state::AppState};
    use axum::{Router, http::header, routing::get};

    fn stUser(iId: i32, sNick: &str) -> UserSummary {
        UserSummary {
            id: iId,
            nick: sNick.to_owned(),
            name: None,
            score: Some(100),
            max_score: Some(100),
            photo: None,
            town: None,
            regdate: None,
            canmod: false,
            candel: false,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    #[test]
    fn authenticated_user_cannot_open_another_users_self_service_form() {
        let optCurrent = Some(stUser(1, "maxcom"));

        assert!(matches!(
            ensure_self_service_actor(&optCurrent, "other"),
            Err(AppError::Forbidden)
        ));
    }

    #[test]
    fn authenticated_owner_passes_self_service_entrypoint() {
        let optCurrent = Some(stUser(1, "maxcom"));

        assert!(ensure_self_service_actor(&optCurrent, "maxcom").is_ok());
    }

    #[test]
    fn reset_password_confirmation_matches_bare_and_encoded_query_key() {
        assert!(bHasRequestParameter(
            Some("reset-password"),
            "reset-password"
        ));
        assert!(bHasRequestParameter(
            Some("%72eset-password=&offset=30"),
            "reset-password"
        ));
        assert!(!bHasRequestParameter(
            Some("reset_password=true"),
            "reset-password"
        ));
        assert!(bHasRequestParameter(Some("year-stats"), "year-stats"));
        assert!(!bHasRequestParameter(Some("year_stats=true"), "year-stats"));
    }

    #[test]
    fn edit_profile_restrictions_follow_java_order() {
        assert_eq!(
            optEditProfileInfoRestriction(true, true, true, true),
            Some("установлен режим только для чтения")
        );
        assert!(optEditProfileInfoRestriction(false, false, false, false).is_none());
    }

    #[test]
    fn profile_url_and_markup_match_original_form_contract() {
        assert_eq!(
            optFixedProfileUrl(Some("www.example.org/path")).unwrap(),
            Some("http://www.example.org/path".into())
        );
        assert!(optFixedProfileUrl(Some(" example.org ")).is_err());
        assert!(optFixedProfileUrl(Some("javascript:alert(1)")).is_err());
        assert_eq!(sMarkupIdFromForm("markdown"), "MARKDOWN");
        assert_eq!(sMarkupIdFromForm("ntobr"), "BBCODE_ULB");
        assert_eq!(sMarkupIdFromForm("lorcode"), "BBCODE_TEX");
    }

    #[tokio::test]
    async fn anonymous_settings_and_edit_gets_forbidden_before_database_lookup() {
        let oPool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy test pool");
        let stState = AppState::new(
            StConfig {
                host: "127.0.0.1".to_owned(),
                port: 0,
                database_url: "postgres://unused:unused@127.0.0.1:1/unused".to_owned(),
                public_url: "http://127.0.0.1".to_owned(),
                ws_url: "ws://127.0.0.1/".to_owned(),
                static_dir: "static".to_owned(),
                upload_dir: "uploads".to_owned(),
                site_secret: "test-site-secret-test-site-secret".to_owned(),
                opensearch_url: None,
                captcha_public_key: None,
                captcha_private_key: None,
                captcha_verify_url: "https://hcaptcha.com/siteverify".to_owned(),
                admin_email: None,
                smtp_host: "localhost".to_owned(),
                smtp_port: 25,
                smtp_helo_name: "localhost".to_owned(),
                telegram_token: None,
                fallback_proxy_url: None,
                enable_background_jobs: false,
                clean_old_userpics: false,
                trusted_proxy_cidrs: Vec::new(),
                page_size: 30,
                enable_hsts: false,
                enable_dev_bypasses: false,
            },
            oPool,
        );
        let cApp = Router::new()
            .route("/people/{nick}/settings", get(settings))
            .route("/people/{nick}/edit", get(edit_profile_form))
            .with_state(stState);
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let stAddress = stListener.local_addr().expect("listener address");
        let hServer = tokio::spawn(async move {
            axum::serve(stListener, cApp)
                .await
                .expect("test server must serve");
        });
        let cClient = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test client");

        for sPath in ["/people/maxcom/settings?tab=display", "/people/maxcom/edit"] {
            let stResponse = cClient
                .get(format!("http://{stAddress}{sPath}"))
                .send()
                .await
                .expect("request to test router");
            assert_eq!(stResponse.status(), reqwest::StatusCode::FORBIDDEN);
            assert!(stResponse.headers().get(header::LOCATION).is_none());
        }

        hServer.abort();
    }
}
