use crate::{
    application::{
        boxlet::CBoxletService,
        tag::{
            CTagTopicListService, bTagSectionHasNext, dtTagTopicCountDeadline,
            iTagTopicCountOrFallback, optCountTagTopicsBeforeDeadline, optTagSectionPreviousOffset,
            sTagSectionUrl,
        },
    },
    auth::CurrentUser,
    domain::{
        boxlet::model::StTagCloudItem,
        tag::model::{EnTagSectionOutcome, EnTagSectionTopics, StTagForumTopic},
        topic::options::TrTopicReindexQueue,
    },
    error::{AppError, Result},
    infra::{
        opensearch::tag_topic_count::CTagTopicCountOpenSearchRepository,
        postgres::{
            boxlet_repository::CBoxletPgRepository,
            tag_topic_list_repository::CTagTopicListPgRepository,
        },
        search_queue::CSearchQueueSender,
    },
    models::TopicSummary,
    request_timezone::stRequestTimezone,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, TimeZone};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Template)]
#[template(path = "tags.html")]
struct TagsTemplate {
    first_letters: Vec<TagsFirstLetterView>,
    tags: Vec<TagsListItemView>,
    tagcloud: Vec<StTagCloudItem>,
    is_moderator: bool,
}

#[derive(Debug, Clone)]
struct TagsFirstLetterView {
    value: String,
    url: String,
    selected: bool,
}

#[derive(Debug, Clone)]
struct TagsListItemView {
    value: String,
    counter: i32,
    url: Option<String>,
    edit_url: String,
    delete_url: String,
}

#[derive(Debug, Clone)]
struct TagSectionGroup {
    section_prefix: String,
    section_name: String,
    topic_columns: Vec<Vec<TagDateGroup>>,
    full_news: Vec<crate::routes::topics::NewsTopicView>,
    gallery: Vec<TagGalleryItem>,
    newest_date: Option<chrono::DateTime<chrono::Utc>>,
    add_url: Option<String>,
    add_reason: String,
    add_label: String,
    more_url: Option<String>,
    more_label: String,
}

#[derive(Debug, Clone)]
struct TagDateGroup {
    label: String,
    topics: Vec<TopicSummary>,
}

#[derive(Debug, Clone)]
struct TagGalleryItem {
    topic: TopicSummary,
    medium_url: String,
    srcset: String,
}

#[derive(Template)]
#[template(path = "tag_page.html")]
struct TagPageTemplate {
    tag: String,
    tag_query_value: String,
    title: String,
    counter: i64,
    sections: Vec<TagSectionGroup>,
    related_tags: Vec<TagLink>,
    synonyms: Vec<TagSynonymView>,
    show_favorite_button: bool,
    show_unfavorite_button: bool,
    show_ignore_button: bool,
    show_unignore_button: bool,
    authorized: bool,
    show_delete: bool,
    favorites_count: i64,
    ignored_count: i64,
    csrf_token: String,
}

#[derive(Debug, Clone)]
struct TagLink {
    name: String,
    url: String,
}

#[derive(Debug, Clone)]
struct TagSynonymView {
    name: String,
    url: String,
    delete_url: String,
}

#[derive(Debug, Clone)]
struct StTagSectionLinkView {
    sName: String,
    sUrl: String,
    bSelected: bool,
}

#[derive(Debug)]
struct StTagForumTopicView {
    stTopic: StTagForumTopic,
    sLastPageUrl: String,
    sGroupUrl: String,
    iVisibleCommentCount: i32,
    bCommentsClosed: bool,
}

impl StTagForumTopicView {
    fn sTitlePlain(&self) -> String {
        self.stTopic.sTitlePlain()
    }

    fn vecTags(&self) -> Vec<&str> {
        self.stTopic.vecTags()
    }
}

#[derive(Template)]
#[template(path = "tag_topics.html")]
struct StTagTopicsTemplate {
    sTitle: String,
    sTagTitle: String,
    sTag: String,
    sTagQueryValue: String,
    sTagUrl: String,
    vecSectionLinks: Vec<StTagSectionLinkView>,
    optAddUrl: Option<String>,
    sAddReason: String,
    bAuthorized: bool,
    bShowFavorite: bool,
    bShowUnfavorite: bool,
    bShowIgnore: bool,
    bShowUnignore: bool,
    iFavoritesCount: i64,
    iIgnoreCount: i64,
    iCounter: i64,
    vecTopics: Vec<crate::routes::topics::NewsTopicView>,
    optPreviousLink: Option<String>,
    optNextLink: Option<String>,
    sCsrfToken: String,
}

#[derive(Template)]
#[template(path = "tag_topics_forum.html")]
struct StTagTopicsForumTemplate {
    sTitle: String,
    sTagTitle: String,
    sTagQueryValue: String,
    sTagUrl: String,
    vecSectionLinks: Vec<StTagSectionLinkView>,
    optAddUrl: Option<String>,
    sAddReason: String,
    bAuthorized: bool,
    bShowFavorite: bool,
    bShowUnfavorite: bool,
    bShowIgnore: bool,
    bShowUnignore: bool,
    iFavoritesCount: i64,
    iIgnoreCount: i64,
    iCounter: i64,
    vecTopics: Vec<StTagForumTopicView>,
    bOldTracker: bool,
    optPreviousLink: Option<String>,
    optNextLink: Option<String>,
}

/// TagController's `/tags` path is shared with `showTagListHandlerJSON`,
/// disambiguated in Java by `params = Array("term")`; axum has no
/// path+query-based dispatch, so branch on `term`'s presence here instead.
pub async fn all_tags(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    stRequest: Request,
) -> Result<axum::response::Response> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    // Spring's `params = "term"` condition tests parameter presence, so an
    // explicitly empty `?term=` still selects the JSON autocomplete handler.
    if let Some(sTerm) = crate::form::get(&vecParameters, "term") {
        return Ok(Json(tag_autocomplete(&state, sTerm).await?).into_response());
    }
    let first_letters = vecTagsFirstLetters(&state, None).await?;
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(state.pool.clone()),
        &state.config.upload_dir,
    );
    Ok(Html(
        TagsTemplate {
            first_letters,
            tags: Vec::new(),
            tagcloud: cService.vecTagCloud().await?,
            is_moderator: user.is_some_and(|stUser| stUser.canmod),
        }
        .render()?,
    )
    .into_response())
}

/// TagService.suggestTagsByPrefix/TagDao.getTopTagsByPrefix: union of real
/// tag values and synonyms matching `prefix%` with counter>=2, top 10 by
/// counter, alphabetically sorted, then filtered by `isGoodTag` in the
/// controller.
async fn tag_autocomplete(state: &AppState, term: &str) -> Result<Vec<String>> {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("{escaped}%");
    let mut tags: Vec<String> = sqlx::query_scalar(
        r#"SELECT value FROM (
             SELECT s.value, v.counter FROM tags_synonyms s JOIN tags_values v ON s.tagid=v.id WHERE s.value LIKE $1
             UNION ALL
             SELECT value, counter FROM tags_values WHERE value LIKE $1
           ) j
           WHERE counter>=2
           ORDER BY counter DESC
           LIMIT 10"#,
    )
    .bind(&pattern)
    .fetch_all(&state.pool)
    .await?;
    tags.retain(|t| is_good_tag(t));
    tags.sort();
    Ok(tags)
}

pub async fn tags_by_letter(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(first_letter): Path<String>,
) -> Result<Html<String>> {
    let escaped_prefix = first_letter
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let prefix = format!("{escaped_prefix}%");
    let is_moderator = user.as_ref().is_some_and(|stUser| stUser.canmod);
    let threshold = if user
        .as_ref()
        .is_some_and(|stUser| stUser.canmod || stUser.corrector)
    {
        1
    } else {
        2
    };
    let rows: Vec<(String, i32)> = sqlx::query_as(
        r#"SELECT value, counter
           FROM tags_values
           WHERE value LIKE $1 ESCAPE '\' AND counter >= $2
           ORDER BY value"#,
    )
    .bind(prefix)
    .bind(threshold)
    .fetch_all(&state.pool)
    .await?;
    if rows.is_empty() {
        return Err(AppError::NotFound);
    }
    let encoded_letter = urlencoding::encode(&first_letter);
    let tags = rows
        .into_iter()
        .map(|(value, counter)| {
            let encoded_tag = urlencoding::encode(&value);
            let encoded_query_tag = urlencoding::encode(&value);
            TagsListItemView {
                url: is_good_tag(&value).then(|| format!("/tag/{encoded_tag}")),
                edit_url: format!(
                    "/tags/change?firstLetter={encoded_letter}&tagName={encoded_query_tag}"
                ),
                delete_url: format!(
                    "/tags/delete?firstLetter={encoded_letter}&tagName={encoded_query_tag}"
                ),
                value,
                counter,
            }
        })
        .collect();
    Ok(Html(
        TagsTemplate {
            first_letters: vecTagsFirstLetters(&state, Some(&first_letter)).await?,
            tags,
            tagcloud: Vec::new(),
            is_moderator,
        }
        .render()?,
    ))
}

async fn vecTagsFirstLetters(
    state: &AppState,
    current_letter: Option<&str>,
) -> Result<Vec<TagsFirstLetterView>> {
    let values: Vec<String> = sqlx::query_scalar(
        r#"SELECT DISTINCT lower(substr(value, 1, 1)) AS firstchar
           FROM tags_values
           WHERE counter > 0
           ORDER BY firstchar"#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(values
        .into_iter()
        .map(|value| TagsFirstLetterView {
            url: format!("/tags/{}", urlencoding::encode(&value)),
            selected: current_letter == Some(value.as_str()),
            value,
        })
        .collect())
}

pub async fn old_tags_redirect() -> impl IntoResponse {
    (StatusCode::FOUND, [(header::LOCATION, "/tags")])
}

/// Per-section topic caps, matching TagPageController's TotalNewsCount(21)/
/// ForumTopicCount(20)/GalleryCount(6) (polls/articles use the forum count
/// in the original too - no dedicated constant).
fn section_topic_limit(section_prefix: &str) -> i64 {
    match section_prefix {
        "news" => 21,
        "gallery" => 6,
        _ => 20,
    }
}

const TAG_SECTION_ORDER: &[(&str, &str)] = &[
    ("news", "Новости"),
    ("forum", "Форум"),
    ("polls", "Опросы"),
    ("gallery", "Галерея"),
    ("articles", "Статьи"),
];

fn sTagDatePartition(
    dtDate: chrono::DateTime<chrono::Utc>,
    stTimezone: chrono_tz::Tz,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> String {
    let dtLocalNow = dtNow.with_timezone(&stTimezone);
    let dtToday = stTimezone
        .with_ymd_and_hms(
            dtLocalNow.year(),
            dtLocalNow.month(),
            dtLocalNow.day(),
            0,
            0,
            0,
        )
        .earliest()
        .expect("timezone start of day");
    let dtYesterday = dtToday - chrono::Duration::days(1);
    // `withDayOfMonth(1).minusYears(1)`: the first day of the current
    // month one year ago (not "now minus 365 days").
    let dtYearAgo = stTimezone
        .with_ymd_and_hms(dtLocalNow.year() - 1, dtLocalNow.month(), 1, 0, 0, 0)
        .earliest()
        .expect("timezone start of month");
    let dtLocal = dtDate.with_timezone(&stTimezone);
    if dtDate > dtToday.with_timezone(&chrono::Utc) {
        "Сегодня".to_owned()
    } else if dtDate > dtYesterday.with_timezone(&chrono::Utc) {
        "Вчера".to_owned()
    } else if dtDate > dtYearAgo.with_timezone(&chrono::Utc) {
        const MONTHS: [&str; 12] = [
            "Январь",
            "Февраль",
            "Март",
            "Апрель",
            "Май",
            "Июнь",
            "Июль",
            "Август",
            "Сентябрь",
            "Октябрь",
            "Ноябрь",
            "Декабрь",
        ];
        format!("{} {}", MONTHS[dtLocal.month0() as usize], dtLocal.year())
    } else {
        dtLocal.year().to_string()
    }
}

fn vecGroupedTagTopics(vecValues: Vec<(String, TopicSummary)>) -> Vec<TagDateGroup> {
    let mut vecGroups: Vec<TagDateGroup> = Vec::new();
    for (sLabel, stTopic) in vecValues {
        if let Some(stLast) = vecGroups.last_mut()
            && stLast.label == sLabel
        {
            stLast.topics.push(stTopic);
        } else {
            vecGroups.push(TagDateGroup {
                label: sLabel,
                topics: vec![stTopic],
            });
        }
    }
    vecGroups
}

/// `TopicListTools.split`: insert one spacer between date groups before
/// splitting so a heading and its first topic are not separated merely by
/// the midpoint calculation, then group each half independently.
fn vecTagTopicColumns(
    vecTopics: Vec<TopicSummary>,
    stTimezone: chrono_tz::Tz,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> Vec<Vec<TagDateGroup>> {
    if vecTopics.is_empty() {
        return Vec::new();
    }
    let mut vecSlots: Vec<Option<(String, TopicSummary)>> = Vec::new();
    let mut optPrevious: Option<String> = None;
    for stTopic in vecTopics {
        let sLabel = sTagDatePartition(stTopic.postdate, stTimezone, dtNow);
        if optPrevious.as_deref().is_some_and(|sOld| sOld != sLabel) {
            vecSlots.push(None);
        }
        optPrevious = Some(sLabel.clone());
        vecSlots.push(Some((sLabel, stTopic)));
    }
    let iSplit = vecSlots.len().div_ceil(2);
    let vecSecond = vecSlots.split_off(iSplit);
    [vecSlots, vecSecond]
        .into_iter()
        .map(|vecColumn| vecGroupedTagTopics(vecColumn.into_iter().flatten().collect()))
        .collect()
}

/// TagPageController.tagPage: aggregates the tag's topics across all 5
/// sections (news/gallery/forum/polls/articles) on one page instead of a
/// flat single-section list, resolves a synonym redirect if the tag itself
/// has no direct topics, lists sibling synonyms, and surfaces
/// favorite/ignore-tag button state - none of which the previous flat
/// listing did.
pub async fn tag_page(
    State(stState): State<AppState>,
    Path(sTag): Path<String>,
    stJar: CookieJar,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    stHeaders: HeaderMap,
    stRequest: Request,
) -> Result<axum::response::Response> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &stHeaders,
        &stState.config.trusted_proxy_cidrs,
    )
    .to_string();
    let Some(sRawSection) = crate::form::get(&vecParameters, "section") else {
        return aggregate_tag_page(stState, sTag, stJar, optUser, sRemoteIp, sCsrfToken).await;
    };

    // `defaultValue="0"` applies to an explicitly empty value too; a
    // non-empty type mismatch is handled by the global bad-parameter view.
    let iSectionId = iTagSectionParameter(sRawSection, "section")?;
    let sRawOffset = crate::form::get(&vecParameters, "offset").unwrap_or("0");
    let iRawOffset = iTagSectionParameter(sRawOffset, "offset")?;

    if !is_good_tag(&sTag) {
        return Ok(stTagNameUserErrorResponse(&sTag));
    }

    let cService = CTagTopicListService::new(
        CTagTopicListPgRepository::new(stState.pool.clone()),
        CTagTopicCountOpenSearchRepository::new(
            stState.config.opensearch_url.clone(),
            stState.http.clone(),
        ),
    );
    let enOutcome = cService
        .enSectionPage(
            &sTag,
            iSectionId,
            iRawOffset,
            optUser.as_ref().map(|stUser| stUser.id),
        )
        .await?;
    let EnTagSectionOutcome::Page(stPage) = enOutcome else {
        let EnTagSectionOutcome::Redirect {
            sMainTag,
            iSectionId,
        } = enOutcome
        else {
            unreachable!("covered tag-section outcomes")
        };
        let sLocation = sTagSectionUrl(&sMainTag, iSectionId, 0);
        return Ok(crate::routes::stSeeOtherRedirect(sLocation));
    };

    let iItems = stPage.enTopics.iLen();
    let optPreviousLink = optTagSectionPreviousOffset(stPage.iOffset, stPage.iPageSize)
        .map(|iOffset| sTagSectionUrl(&sTag, stPage.stSection.iId, iOffset));
    let optNextLink = bTagSectionHasNext(stPage.iOffset, iItems, stPage.iPageSize).then(|| {
        sTagSectionUrl(
            &sTag,
            stPage.stSection.iId,
            stPage.iOffset + stPage.iPageSize,
        )
    });
    let sTagUrl = sTagSectionUrl(&sTag, 0, 0);
    let sTagQueryValue = urlencoding::encode(&sTag).into_owned();
    let vecSectionLinks = stPage
        .vecSections
        .iter()
        .map(|stSection| StTagSectionLinkView {
            sName: stSection.sName.clone(),
            sUrl: sTagSectionUrl(&sTag, stSection.iId, 0),
            bSelected: stSection.iId == stPage.stSection.iId,
        })
        .collect::<Vec<_>>();
    let sTagTitle = capitalize_first(&sTag);
    let sTitle = format!("{sTagTitle} ({})", stPage.stSection.sName);
    let stPostingIdentity =
        crate::application::auth::stResolvePostingIdentity(&stState, optUser.as_ref(), None, None)
            .await?
            .stIdentity;
    let stPostingUser = &stPostingIdentity.stUser;
    let stPostingActor = crate::domain::topic::posting::StAddTopicActor {
        optUserId: Some(stPostingUser.id),
        bAnonymous: !stPostingIdentity.bAuthorized,
        bModerator: stPostingUser.canmod,
        bCorrector: stPostingUser.corrector,
        bBlocked: stPostingUser.blocked.unwrap_or(false),
        iScore: stPostingUser.score.unwrap_or(0),
    };
    let stPostingPermission = crate::application::topic::posting::CAddTopicService::new(
        crate::infra::postgres::add_topic_repository::CAddTopicPgRepository::new(
            stState.pool.clone(),
        ),
    )
    .stCheckRestriction(
        stPage.stSection.iTopicsRestriction,
        stPostingActor,
        &sRemoteIp,
    )
    .await?;
    let optPostingReason = stPostingPermission.optReason;
    let optAddUrl = optPostingReason.as_ref().is_none().then(|| {
        format!(
            "/add-section.jsp?section={}&tag={}",
            stPage.stSection.iId,
            urlencoding::encode(&sTag)
        )
    });
    let sAddReason = optPostingReason.unwrap_or_default();
    let bAuthorized = optUser.is_some();
    let bModerator = optUser.as_ref().is_some_and(|stUser| stUser.canmod);
    let bShowFavorite = bAuthorized && !stPage.stViewerState.bFavorite;
    let bShowUnfavorite = bAuthorized && stPage.stViewerState.bFavorite;
    let bShowIgnore = bAuthorized && !bModerator && !stPage.stViewerState.bIgnored;
    let bShowUnignore = bAuthorized && !bModerator && stPage.stViewerState.bIgnored;

    match stPage.enTopics {
        EnTagSectionTopics::Feed(vecTopics) => {
            let mut vecTopics = crate::routes::topics::prepare_news_topics_for_viewer(
                &stState,
                vecTopics,
                true,
                &optUser,
                &sCsrfToken,
            )
            .await?;
            // tag-topics.jsp passes minorAsMajor=true to news.tag.
            for stTopic in &mut vecTopics {
                stTopic.minor = false;
            }
            Ok(Html(
                StTagTopicsTemplate {
                    sTitle,
                    sTagTitle,
                    sTag,
                    sTagQueryValue,
                    sTagUrl,
                    vecSectionLinks,
                    optAddUrl,
                    sAddReason,
                    bAuthorized,
                    bShowFavorite,
                    bShowUnfavorite,
                    bShowIgnore,
                    bShowUnignore,
                    iFavoritesCount: stPage.stViewerState.iFavoritesCount,
                    iIgnoreCount: stPage.stViewerState.iIgnoreCount,
                    iCounter: stPage.iCounter,
                    vecTopics,
                    optPreviousLink,
                    optNextLink,
                    sCsrfToken,
                }
                .render()?,
            )
            .into_response())
        }
        EnTagSectionTopics::Forum(vecTopics) => {
            let iMessages = stPage.stProfile.iMessages;
            let vecTopics = vecTopics
                .into_iter()
                .map(|stTopic| StTagForumTopicView {
                    sLastPageUrl: stTopic.sLastPageUrl(iMessages),
                    sGroupUrl: stTopic.sGroupUrl(),
                    iVisibleCommentCount: stTopic.iVisibleCommentCount(),
                    bCommentsClosed: stTopic.bCommentsClosed(),
                    stTopic,
                })
                .collect();
            Ok(Html(
                StTagTopicsForumTemplate {
                    sTitle,
                    sTagTitle,
                    sTagQueryValue,
                    sTagUrl,
                    vecSectionLinks,
                    optAddUrl,
                    sAddReason,
                    bAuthorized,
                    bShowFavorite,
                    bShowUnfavorite,
                    bShowIgnore,
                    bShowUnignore,
                    iFavoritesCount: stPage.stViewerState.iFavoritesCount,
                    iIgnoreCount: stPage.stViewerState.iIgnoreCount,
                    iCounter: stPage.iCounter,
                    vecTopics,
                    bOldTracker: stPage.stProfile.bOldTracker,
                    optPreviousLink,
                    optNextLink,
                }
                .render()?,
            )
            .into_response())
        }
    }
}

/// The live pinned Spring/Jetty stack answers malformed numeric parameters
/// on `TagTopicListController` with HTTP 400. This is distinct from an
/// explicit `ServletParameterException`, whose themed error page is a 404.
fn iTagSectionParameter(sRawValue: &str, sName: &str) -> Result<i32> {
    if sRawValue.trim().is_empty() {
        return Ok(0);
    }

    sRawValue
        .trim()
        .parse::<i32>()
        .map_err(|_| AppError::BadRequest(format!("Некорректное значение параметра `{sName}`")))
}

#[derive(Template)]
#[template(path = "topic_edit_user_error.html")]
struct StTagNameUserErrorTemplate<'a> {
    exception_class: &'static str,
    message: &'a str,
}

fn stTagNameUserErrorResponse(sTag: &str) -> axum::response::Response {
    let sMessage = format!("Некорректный тег: '{sTag}'");
    match (StTagNameUserErrorTemplate {
        exception_class: "ru.org.linux.user.UserErrorException",
        message: &sMessage,
    })
    .render()
    {
        Ok(sBody) => (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response(),
        Err(stError) => AppError::Template(stError).into_response(),
    }
}

async fn aggregate_tag_page(
    state: AppState,
    tag: String,
    stJar: CookieJar,
    user: Option<crate::models::UserSummary>,
    sRemoteIp: String,
    sCsrfToken: String,
) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;

    if !is_good_tag(&tag) {
        return Err(AppError::NotFound);
    }

    // TagPageController starts both OpenSearch futures before assembling the
    // page and gives them the same absolute 500 ms deadline. The aggregate
    // count deliberately has no section filter.
    let dtSearchDeadline = dtTagTopicCountDeadline();
    let cCountRepository = CTagTopicCountOpenSearchRepository::new(
        state.config.opensearch_url.clone(),
        state.http.clone(),
    );
    let sCountTag = tag.clone();
    let dtCountDeadline = dtSearchDeadline;
    let stCountTask = tokio::spawn(async move {
        optCountTagTopicsBeforeDeadline(&cCountRepository, &sCountTag, None, dtCountDeadline).await
    });
    let stRelatedState = state.clone();
    let sRelatedTag = tag.clone();
    let dtRelatedDeadline = dtSearchDeadline;
    let stRelatedTask = tokio::spawn(async move {
        match tokio::time::timeout_at(
            dtRelatedDeadline,
            crate::search_index::vecRelatedTags(&stRelatedState, &sRelatedTag),
        )
        .await
        {
            Ok(Ok(vecTags)) => vecTags,
            Ok(Err(stError)) => {
                tracing::warn!(error = %stError, tag = %sRelatedTag, "unable to find related tags");
                Vec::new()
            }
            Err(_) => {
                tracing::warn!(tag = %sRelatedTag, "tag related search timed out");
                Vec::new()
            }
        }
    });

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    let tag_row: Option<(i32, i64)> =
        sqlx::query_as("SELECT id, counter::bigint FROM tags_values WHERE value=$1")
            .bind(&tag)
            .fetch_optional(&state.pool)
            .await?;

    let Some((tag_id, persisted_counter)) =
        tag_row.filter(|(_, counter)| is_moderator || *counter > 0)
    else {
        // No direct tag (or a moderator-only zero-count tag hidden from
        // regular users) - check whether `tag` is actually a synonym
        // pointing at a real tag, and redirect there if so.
        let synonym_target: Option<String> = sqlx::query_scalar(
            "SELECT tv.value FROM tags_synonyms ts JOIN tags_values tv ON tv.id=ts.tagid WHERE ts.value=$1",
        )
        .bind(&tag)
        .fetch_optional(&state.pool)
        .await?;
        return match synonym_target {
            Some(main_tag) => Ok(crate::routes::stSeeOtherRedirect(format!(
                "/tag/{}",
                urlencoding::encode(&main_tag)
            ))),
            None => Err(AppError::NotFound),
        };
    };

    let stTimezone = stRequestTimezone(&stJar);
    let dtNow = chrono::Utc::now();
    let optViewerId = user.as_ref().map(|stUser| stUser.id);
    let bViewerAuthorized = user.is_some();
    let mut sections = Vec::new();
    for (prefix, name) in TAG_SECTION_ORDER {
        let limit = section_topic_limit(prefix);
        let (section_id, restriction): (i32, i32) = sqlx::query_as(
            r#"SELECT id,COALESCE(restrict_topics,-9999) FROM sections WHERE CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END=$1"#,
        ).bind(prefix).fetch_one(&state.pool).await?;
        let topics = sqlx::query_as::<_, TopicSummary>(
            r#"SELECT t.id, t.title, t.url,
                      CASE WHEN t.moderate AND t.commitdate IS NOT NULL THEN t.commitdate ELSE t.postdate END AS postdate,
                      t.lastmod, u.id AS author_id, u.nick AS author,
                      g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                      s.id AS section_id, s.name AS section_name,
                      $1::text AS section_prefix,
                      t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                      (SELECT string_agg(tv2.value, ',' ORDER BY tv2.value)
                         FROM tags tg2 JOIN tags_values tv2 ON tv2.id=tg2.tagid
                        WHERE tg2.msgid=t.id) AS tags
               FROM topics t
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               JOIN tags tg ON tg.msgid=t.id AND tg.tagid=$2
               WHERE (CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END) = $1
                 AND NOT t.deleted
                 AND ($1='gallery' OR NOT COALESCE(t.draft,false))
                 AND (($1='forum' AND NOT s.moderate)
                   OR ($1<>'forum' AND s.moderate AND t.commitdate IS NOT NULL
                     AND ($1<>'gallery' OR t.moderate)))
                 AND ($1='gallery' OR $3 OR t.open_warnings <= 2)
                 AND ($1<>'forum' OR $4::int IS NULL OR NOT EXISTS (
                   SELECT 1 FROM ignore_list il
                    WHERE il.userid=$4 AND il.ignored=t.userid
                 ))
               ORDER BY CASE WHEN t.moderate AND t.commitdate IS NOT NULL THEN t.commitdate ELSE t.postdate END DESC
               LIMIT $5"#,
        )
        .bind(prefix)
        .bind(tag_id)
        .bind(bViewerAuthorized)
        .bind(optViewerId)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?;
        if !topics.is_empty() {
            let iLoadedTopicCount = topics.len() as i64;
            let newest_date = topics.first().map(|topic| topic.postdate);
            let recent_news = *prefix == "news"
                && topics
                    .first()
                    .is_some_and(|topic| topic.postdate > dtNow - chrono::Duration::days(365));
            let full_news = if recent_news {
                crate::routes::topics::prepare_news_topics(&state, vec![topics[0].clone()], true)
                    .await?
            } else {
                Vec::new()
            };
            let mut gallery = Vec::new();
            if *prefix == "gallery" {
                for topic in &topics {
                    if let Some(image) = crate::routes::topics::load_topic_images(&state, topic.id)
                        .await?
                        .into_iter()
                        .next()
                    {
                        gallery.push(TagGalleryItem {
                            topic: topic.clone(),
                            medium_url: image.medium_url.clone(),
                            srcset: crate::routes::topics::topic_image_srcset(&image),
                        });
                    }
                }
            }
            if *prefix == "gallery" && gallery.is_empty() {
                continue;
            }
            let more_url = if *prefix == "gallery" {
                (gallery.len() as i64 == limit).then(|| sTagSectionUrl(&tag, section_id, 0))
            } else {
                (iLoadedTopicCount == limit).then(|| sTagSectionUrl(&tag, section_id, 0))
            };
            let brief_topics = if *prefix == "gallery" {
                Vec::new()
            } else if recent_news {
                topics.into_iter().skip(1).collect()
            } else if *prefix == "news" {
                topics.into_iter().take((limit - 1) as usize).collect()
            } else {
                topics
            };
            let topic_columns = vecTagTopicColumns(brief_topics, stTimezone, dtNow);
            let add_reason = crate::routes::topics::posting_reason_for_port(
                &state,
                restriction,
                &user,
                &sRemoteIp,
            )
            .await?;
            let add_url = add_reason.is_none().then(|| {
                format!(
                    "/add-section.jsp?section={section_id}&tag={}",
                    urlencoding::encode(&tag)
                )
            });
            let add_label = match *prefix {
                "news" => "Добавить новость",
                "gallery" => "Добавить изображение",
                "polls" => "Добавить опрос",
                _ => "Добавить топик",
            }
            .to_string();
            let more_label = match *prefix {
                "news" => "Все новости",
                "gallery" => "Все изображения",
                _ => "Все топики",
            }
            .to_string();
            sections.push(TagSectionGroup {
                section_prefix: prefix.to_string(),
                section_name: name.to_string(),
                topic_columns,
                full_news,
                gallery,
                newest_date,
                add_url,
                add_reason: add_reason.unwrap_or_default(),
                add_label,
                more_url,
                more_label,
            });
        }
    }

    // TagPageController renders the freshest of news/forum first, places
    // polls/gallery/articles in the middle, then renders the other one.
    let mut optNews = sections
        .iter()
        .position(|stSection| stSection.section_prefix == "news")
        .map(|iPosition| sections.remove(iPosition));
    let mut optForum = sections
        .iter()
        .position(|stSection| stSection.section_prefix == "forum")
        .map(|iPosition| sections.remove(iPosition));
    let bNewsFirst = optNews.as_ref().is_some_and(|stNews| {
        stNews.newest_date.is_some_and(|dtNews| {
            dtNews > dtNow - chrono::Duration::days(365)
                || optForum
                    .as_ref()
                    .and_then(|stForum| stForum.newest_date)
                    .is_some_and(|dtForum| dtNews > dtForum)
        })
    });
    let mut vecOrdered = Vec::new();
    if bNewsFirst {
        vecOrdered.extend(optNews.take());
    } else {
        vecOrdered.extend(optForum.take());
    }
    vecOrdered.append(&mut sections);
    if bNewsFirst {
        vecOrdered.extend(optForum.take());
    } else {
        vecOrdered.extend(optNews.take());
    }
    sections = vecOrdered;

    let synonyms: Vec<String> =
        sqlx::query_scalar("SELECT value FROM tags_synonyms WHERE tagid=$1 ORDER BY value")
            .bind(tag_id)
            .fetch_all(&state.pool)
            .await?;
    let synonyms = synonyms
        .into_iter()
        .map(|sName| TagSynonymView {
            url: format!("/tag/{}", urlencoding::encode(&sName)),
            delete_url: format!("/tags/delete?tagName={}", urlencoding::encode(&sName)),
            name: sName,
        })
        .collect();

    let (show_favorite_button, show_unfavorite_button, show_ignore_button, show_unignore_button) =
        match &user {
            Some(u) => {
                let is_fav: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND is_favorite)")
                .bind(u.id).bind(tag_id).fetch_one(&state.pool).await?;
                let is_ignored: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND NOT is_favorite)")
                .bind(u.id).bind(tag_id).fetch_one(&state.pool).await?;
                (
                    !is_fav,
                    is_fav,
                    !is_moderator && !is_ignored,
                    !is_moderator && is_ignored,
                )
            }
            None => (false, false, false, false),
        };
    let favorites_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite")
            .bind(tag_id)
            .fetch_one(&state.pool)
            .await?;
    let ignored_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND NOT is_favorite")
            .bind(tag_id)
            .fetch_one(&state.pool)
            .await?;

    let optSearchCounter = match stCountTask.await {
        Ok(optCount) => optCount,
        Err(stError) => {
            tracing::warn!(error = %stError, tag = %tag, "tag topic count task failed");
            None
        }
    };
    let counter = iTagTopicCountOrFallback(optSearchCounter, persisted_counter);
    let related_tags = match stRelatedTask.await {
        Ok(vecTags) => vecTags,
        Err(stError) => {
            tracing::warn!(error = %stError, tag = %tag, "related tag task failed");
            Vec::new()
        }
    }
    .into_iter()
    .map(|name| TagLink {
        url: format!("/tag/{}", urlencoding::encode(&name)),
        name,
    })
    .collect();

    Ok(Html(
        TagPageTemplate {
            tag: tag.clone(),
            tag_query_value: urlencoding::encode(&tag).into_owned(),
            title: capitalize_first(&tag),
            counter,
            sections,
            related_tags,
            synonyms,
            show_favorite_button,
            show_unfavorite_button,
            show_ignore_button,
            show_unignore_button,
            authorized: user.is_some(),
            show_delete: is_moderator,
            favorites_count,
            ignored_count,
            csrf_token: sCsrfToken,
        }
        .render()?,
    )
    .into_response())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// TagName.isGoodTag: unicode letters/digits/hyphen, optionally with
/// interior dots/spaces/plus, 1-32 chars.
static TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[\p{L}\d-](?:[.\p{L}\d \+-]*[\p{L}\d\+-])?$").expect("tag regex")
});

pub(crate) fn is_good_tag(tag: &str) -> bool {
    // Scala/Java String.length counts UTF-16 code units, not Unicode scalar
    // values. This matters for supplementary-plane letters near the 32-unit
    // boundary.
    let len = tag.encode_utf16().count();
    (1..=32).contains(&len) && TAG_RE.is_match(tag)
}

/// TagName.MaxTagsPerTopic.
pub(crate) const MAX_TAGS_PER_TOPIC: usize = 5;
/// GroupPermissionService.CreateTagScore.
const CREATE_TAG_SCORE: i32 = 200;

/// TagName.parseTags: split on `,`/`|`, trim, lowercase, dedupe.
pub(crate) fn parse_tags(raw: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for part in raw.replace('|', ",").split(',') {
        let tag = part.trim().to_lowercase();
        if !tag.is_empty() && seen.insert(tag.clone()) {
            out.push(tag);
        }
    }
    out
}

/// TagName.parseAndValidateTags: partitions into good/bad tags, requires at
/// least one good tag and no more than `MAX_TAGS_PER_TOPIC`. Bad tags are
/// silently dropped from the *sanitized* result Java saves
/// (`parseAndSanitizeTags`), but their presence alone doesn't error - only
/// count-of-good-tags and "no good tags at all" do.
pub(crate) fn parse_and_validate_tags(raw: &str) -> std::result::Result<Vec<String>, String> {
    let all = parse_tags(raw);
    let good: Vec<String> = all.into_iter().filter(|t| is_good_tag(t)).collect();
    if good.len() > MAX_TAGS_PER_TOPIC {
        return Err(format!(
            "Слишком много тегов (максимум {MAX_TAGS_PER_TOPIC})"
        ));
    }
    if good.is_empty() {
        return Err("Установите теги".to_string());
    }
    Ok(good)
}

/// GroupPermissionService.canCreateTag plus the port's role separation:
/// outside a premoderated section an ordinary user needs score>=200 to mint
/// a brand-new tag; inside one, any authenticated user may. A moderator
/// (`canmod`) may create tags regardless of score, while the narrower
/// corrector role deliberately gets no moderator bypass. Checked only against
/// tags that don't already exist (TagService.getNewTags) - applying an
/// existing tag never requires this.
pub(crate) async fn check_can_create_new_tags(
    state: &AppState,
    tags: &[String],
    user: &crate::models::UserSummary,
    section_premoderated: bool,
) -> Result<()> {
    if can_create_tag_by_role(user.canmod, user.score.unwrap_or(0), section_premoderated) {
        return Ok(());
    }
    let mut new_tags = Vec::new();
    for tag in tags {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tags_values WHERE lower(value)=lower($1) AND counter>0) OR EXISTS(SELECT 1 FROM tags_synonyms WHERE lower(value)=lower($1))",
        )
        .bind(tag)
        .fetch_one(&state.pool)
        .await?;
        if !exists {
            new_tags.push(tag.clone());
        }
    }
    if new_tags.is_empty() {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Вы не можете создавать новые теги ({})",
            new_tags.join(", ")
        )))
    }
}

fn can_create_tag_by_role(canmod: bool, score: i32, section_premoderated: bool) -> bool {
    section_premoderated || canmod || score >= CREATE_TAG_SCORE
}

#[cfg(test)]
mod create_tag_permission_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn stTopic(iId: i32, dtPostdate: chrono::DateTime<Utc>) -> TopicSummary {
        TopicSummary {
            id: iId,
            title: format!("topic-{iId}"),
            url: None,
            postdate: dtPostdate,
            lastmod: None,
            author_id: 1,
            author: "author".to_owned(),
            group_id: 2,
            group_title: "group".to_owned(),
            group_urlname: "general".to_owned(),
            section_id: 2,
            section_name: "Форум".to_owned(),
            section_prefix: "forum".to_owned(),
            comments: 0,
            deleted: false,
            sticky: false,
            resolved: None,
            tags: None,
        }
    }

    #[test]
    fn comma_and_pipe_separated_tags_remain_individual() {
        assert_eq!(
            parse_tags("Rust, Linux | PostgreSQL, rust"),
            ["rust", "linux", "postgresql"]
        );
    }

    #[test]
    fn tag_length_matches_java_utf16_units() {
        let sSupplementaryLetter = "\u{10400}";
        assert!(is_good_tag(&sSupplementaryLetter.repeat(16)));
        assert!(!is_good_tag(&sSupplementaryLetter.repeat(17)));
    }

    #[test]
    fn section_templates_keep_both_original_topic_modes_and_memory_hooks() {
        let sFeed = include_str!("../../templates/tag_topics.html");
        let sForum = include_str!("../../templates/tag_topics_forum.html");
        let sHeader = include_str!("../../templates/tag_topics_header.html");

        assert!(sFeed.contains("{% include \"news_card.html\" %}"));
        assert!(sFeed.contains("tag_memories_form_setup"));
        assert!(sForum.contains("{% if bOldTracker %}"));
        assert!(sForum.contains("class=\"message-table\""));
        assert!(sForum.contains("class=\"tracker-item\""));
        assert!(sHeader.contains("btn-selected"));
        assert!(sHeader.contains("id=\"favsCount\""));
        assert!(sHeader.contains("id=\"ignoreCount\""));
    }

    #[test]
    fn user_filter_autocomplete_waits_for_the_plugin_dependencies() {
        let sTemplate = include_str!("../../templates/user_filter.html");
        assert!(sTemplate.contains("$script.ready(\"plugins\""));
        assert!(sTemplate.contains("$script(\"/js/tagsAutocomplete.js\")"));
        assert!(!sTemplate.contains("<script src=\"/js/tagsAutocomplete.js\""));
    }

    fn sAggregateTagControls(bAuthorized: bool, bFavorite: bool, bIgnored: bool) -> String {
        TagPageTemplate {
            tag: "rust+web".to_owned(),
            tag_query_value: "rust%2Bweb".to_owned(),
            title: "Rust+web".to_owned(),
            counter: 0,
            sections: Vec::new(),
            related_tags: Vec::new(),
            synonyms: Vec::new(),
            show_favorite_button: bAuthorized && !bFavorite,
            show_unfavorite_button: bAuthorized && bFavorite,
            show_ignore_button: bAuthorized && !bIgnored,
            show_unignore_button: bAuthorized && bIgnored,
            authorized: bAuthorized,
            show_delete: false,
            favorites_count: 0,
            ignored_count: 0,
            csrf_token: "csrf-token".to_owned(),
        }
        .render()
        .unwrap()
    }

    #[test]
    fn aggregate_tag_controls_match_java_get_link_contract() {
        let sAdd = sAggregateTagControls(true, false, false);
        assert!(sAdd.contains("<h1><i class=\"icon-tag\"></i> Rust+web</h1>"));
        assert!(!sAdd.contains("Метка: Rust+web"));
        assert!(
            sAdd.contains("id=\"tagFavAdd\" href=\"/user-filter?newFavoriteTagName=rust%2Bweb\"")
        );
        assert!(
            sAdd.contains("id=\"tagIgnore\" href=\"/user-filter?newIgnoreTagName=rust%2Bweb\"")
        );
        assert!(!sAdd.contains("<form"));

        let sSelected = sAggregateTagControls(true, true, true);
        assert!(sSelected.contains("id=\"tagFavAdd\" href=\"/user-filter\" class=\"selected\""));
        assert!(sSelected.contains("id=\"tagIgnore\" href=\"/user-filter\" class=\"selected\""));

        let sAnonymous = sAggregateTagControls(false, false, false);
        assert!(sAnonymous.contains("id=\"tagFavNoth\" href=\"#\""));
        assert!(sAnonymous.contains("id=\"tagIgnNoth\" href=\"#\""));
    }

    #[test]
    fn aggregate_tag_identity_lookup_is_exact_and_keeps_synonym_fallback() {
        let sSource = include_str!("tags.rs");
        let sAggregate = sSource
            .split(concat!("async fn ", "aggregate_tag_page("))
            .nth(1)
            .unwrap()
            .split("fn capitalize_first")
            .next()
            .unwrap();
        assert!(sAggregate.contains("SELECT id, counter::bigint FROM tags_values WHERE value=$1"));
        assert!(!sAggregate.contains(concat!("lower(value)", "=lower($1)")));
        assert!(sAggregate.contains(
            "FROM tags_synonyms ts JOIN tags_values tv ON tv.id=ts.tagid WHERE ts.value=$1"
        ));
        assert!(sAggregate.contains("is_moderator || *counter > 0"));
        assert!(sAggregate.contains("($1='forum' AND NOT s.moderate)"));
        assert!(sAggregate.contains("$1<>'forum' AND s.moderate AND t.commitdate IS NOT NULL"));
        assert!(sAggregate.contains("($1<>'gallery' OR t.moderate)"));
        assert!(sAggregate.contains("($1='gallery' OR $3 OR t.open_warnings <= 2)"));
        assert!(sAggregate.contains("il.userid=$4 AND il.ignored=t.userid"));
        assert!(!sAggregate.contains("$3 OR t.moderate OR NOT s.moderate"));
        assert!(sAggregate.contains("gallery.len() as i64 == limit"));
        assert!(sAggregate.contains("load_topic_images(&state, topic.id)"));
        assert!(include_str!("topics.rs").contains("ORDER BY main DESC, id"));
    }

    #[test]
    fn aggregate_tag_searches_all_sections_and_uses_the_persisted_fallback() {
        let sSource = include_str!("tags.rs");
        let sAggregate = sSource
            .split(concat!("async fn ", "aggregate_tag_page("))
            .nth(1)
            .unwrap()
            .split("fn capitalize_first")
            .next()
            .unwrap();

        assert!(sAggregate.contains(
            "optCountTagTopicsBeforeDeadline(&cCountRepository, &sCountTag, None, dtCountDeadline)"
        ));
        assert!(
            sAggregate.contains("iTagTopicCountOrFallback(optSearchCounter, persisted_counter)")
        );
        assert!(sAggregate.contains("tokio::time::timeout_at("));
        assert!(sAggregate.contains("dtRelatedDeadline"));
        assert!(!sAggregate.contains("std::time::Duration::from_millis(500)"));
    }

    #[test]
    fn moderators_do_not_need_the_score_threshold() {
        assert!(can_create_tag_by_role(true, 0, false));
        assert!(!can_create_tag_by_role(false, CREATE_TAG_SCORE - 1, false));
        assert!(can_create_tag_by_role(false, CREATE_TAG_SCORE, false));
        assert!(can_create_tag_by_role(false, 0, true));
    }

    #[test]
    fn tag_date_partition_matches_java_boundaries_and_two_column_split() {
        let stTimezone = chrono_tz::Europe::Moscow;
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap();
        assert_eq!(
            sTagDatePartition(
                Utc.with_ymd_and_hms(2026, 8, 8, 8, 0, 0).unwrap(),
                stTimezone,
                dtNow
            ),
            "Сегодня"
        );
        assert_eq!(
            sTagDatePartition(
                Utc.with_ymd_and_hms(2026, 8, 7, 8, 0, 0).unwrap(),
                stTimezone,
                dtNow
            ),
            "Вчера"
        );
        assert_eq!(
            sTagDatePartition(
                Utc.with_ymd_and_hms(2026, 7, 1, 8, 0, 0).unwrap(),
                stTimezone,
                dtNow
            ),
            "Июль 2026"
        );
        assert_eq!(
            sTagDatePartition(
                Utc.with_ymd_and_hms(2025, 7, 1, 8, 0, 0).unwrap(),
                stTimezone,
                dtNow
            ),
            "2025"
        );

        let vecColumns = vecTagTopicColumns(
            vec![
                stTopic(1, Utc.with_ymd_and_hms(2026, 8, 8, 8, 0, 0).unwrap()),
                stTopic(2, Utc.with_ymd_and_hms(2026, 8, 8, 7, 0, 0).unwrap()),
                stTopic(3, Utc.with_ymd_and_hms(2026, 8, 8, 6, 0, 0).unwrap()),
                stTopic(4, Utc.with_ymd_and_hms(2026, 8, 7, 8, 0, 0).unwrap()),
                stTopic(5, Utc.with_ymd_and_hms(2026, 8, 7, 7, 0, 0).unwrap()),
                stTopic(6, Utc.with_ymd_and_hms(2026, 7, 1, 8, 0, 0).unwrap()),
            ],
            stTimezone,
            dtNow,
        );
        assert_eq!(vecColumns.len(), 2);
        assert_eq!(vecColumns[0][0].label, "Сегодня");
        assert_eq!(vecColumns[0][0].topics.len(), 3);
        assert_eq!(vecColumns[1][0].label, "Вчера");
        assert_eq!(vecColumns[1][1].label, "Июль 2026");
    }
}

fn first_letter_of(tag: &str) -> String {
    tag.chars()
        .next()
        .map(|c| c.to_lowercase().to_string())
        .unwrap_or_default()
}

async fn get_tag_id(pool: &sqlx::PgPool, name: &str) -> Result<Option<i32>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)")
            .bind(name)
            .fetch_optional(pool)
            .await?,
    )
}

#[derive(Deserialize)]
pub struct TagChangeQuery {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: String,
}

fn sTagFormErrorsHtml(vecErrors: &[String]) -> String {
    if vecErrors.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"error\">{}</div>",
            vecErrors
                .iter()
                .map(|sError| html_escape::encode_text(sError).into_owned())
                .collect::<Vec<_>>()
                .join("<br>")
        )
    }
}

fn stRenderChangeTagForm(
    sOldTagName: &str,
    sTagName: &str,
    sFirstLetter: &str,
    sCsrfToken: &str,
    vecErrors: &[String],
) -> Result<Html<String>> {
    let sTitle = format!("Изменение тега {sOldTagName}");
    let sAction = format!(
        "/tags/change?firstLetter={}",
        urlencoding::encode(sFirstLetter)
    );
    let sCancelUrl = format!("/tags/{}", urlencoding::encode(sFirstLetter));
    let sContentHtml = format!(
        r#"
<h1>{title}</h1>
<form method="post" action="{action}">
<input type="hidden" name="csrf" value="{csrf_token}">
{errors}
<input type="hidden" name="oldTagName" value="{old_attr}">
Старое название: {old_text}<br>
<label for="tagName">Новое название:</label>
<input id="tagName" name="tagName" value="{tag_attr}" required style="width: 40em" autofocus>
<div class="form-actions">
<button type="submit" class="btn btn-primary">Изменить</button>
<button type="button" class="btn btn-default" onclick="window.location='{cancel_url}'">Отменить</button>
</div>
</form>
"#,
        title = html_escape::encode_text(&sTitle),
        action = html_escape::encode_double_quoted_attribute(&sAction),
        csrf_token = html_escape::encode_double_quoted_attribute(sCsrfToken),
        errors = sTagFormErrorsHtml(vecErrors),
        old_attr = html_escape::encode_double_quoted_attribute(sOldTagName),
        old_text = html_escape::encode_text(sOldTagName),
        tag_attr = html_escape::encode_double_quoted_attribute(sTagName),
        cancel_url = html_escape::encode_double_quoted_attribute(&sCancelUrl),
    );
    Ok(Html(crate::routes::sRenderLegacyContent(
        &sTitle,
        sContentHtml,
    )?))
}

pub async fn change_form(
    CurrentUser(user): CurrentUser,
    Query(q): Query<TagChangeQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    stRenderChangeTagForm(
        &q.tag_name,
        &q.tag_name,
        q.first_letter.as_deref().unwrap_or(""),
        &csrf_token,
        &[],
    )
}

#[derive(Deserialize)]
pub struct TagChangeForm {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
    #[serde(rename = "oldTagName")]
    pub old_tag_name: String,
    #[serde(rename = "tagName")]
    pub tag_name: String,
}

pub async fn change_tag(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<TagChangePostQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<TagChangeForm>,
) -> Result<Response> {
    let moderator = user
        .as_ref()
        .filter(|u| u.canmod)
        .ok_or(AppError::Forbidden)?;
    let old_tag_name = form.old_tag_name.as_str();
    let tag_name = form.tag_name.as_str();
    let optOldTagId = get_tag_id(&state.pool, old_tag_name).await?;
    let bGoodTag = is_good_tag(tag_name);
    let bNewTagExists = if bGoodTag {
        get_tag_id(&state.pool, tag_name).await?.is_some()
    } else {
        false
    };
    let vecErrors = vecChangeTagErrors(optOldTagId.is_some(), bNewTagExists, tag_name);
    if !vecErrors.is_empty() {
        let sFirstLetter = q
            .first_letter
            .as_deref()
            .or(form.first_letter.as_deref())
            .unwrap_or("");
        return Ok(stRenderChangeTagForm(
            old_tag_name,
            tag_name,
            sFirstLetter,
            &csrf_token,
            &vecErrors,
        )?
        .into_response());
    }
    let old_tag_id = optOldTagId.expect("validated existing tag");

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM tags_synonyms WHERE value=$1")
        .bind(tag_name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE tags_values SET value=$2 WHERE id=$1")
        .bind(old_tag_id)
        .bind(tag_name)
        .execute(&mut *tx)
        .await?;
    // TagModificationService.change calls searchQueueSender.updateMessage
    // from inside localTx. A queue failure therefore rolls this rename back.
    let vecTopicIds: Vec<i32> = sqlx::query_scalar("SELECT msgid FROM tags WHERE tagid=$1")
        .bind(old_tag_id)
        .fetch_all(&mut *tx)
        .await?;
    vReindexTopicIds(&state, &vecTopicIds).await?;
    tx.commit().await?;
    tracing::info!(
        old_tag = %old_tag_name,
        new_tag = %tag_name,
        moderator = %moderator.nick,
        "tag changed"
    );

    Ok(crate::routes::stFoundRedirect(format!(
        "/tags/{}",
        urlencoding::encode(&first_letter_of(tag_name))
    )))
}

#[derive(Default, Deserialize)]
pub struct TagChangePostQuery {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
}

fn vecChangeTagErrors(bOldTagExists: bool, bNewTagExists: bool, sTagName: &str) -> Vec<String> {
    let mut vecErrors = Vec::new();
    if !bOldTagExists {
        vecErrors.push("Тега с таким именем не существует!".into());
    }
    if !is_good_tag(sTagName) {
        vecErrors.push(format!("Некорректный тег: '{sTagName}'"));
    } else if bNewTagExists {
        vecErrors.push("Тег с таким именем уже существует!".into());
    }
    vecErrors
}

async fn vReindexTopicIds(state: &AppState, vecTopicIds: &[i32]) -> Result<()> {
    let cQueue = CSearchQueueSender::new(
        state.config.opensearch_url.as_deref(),
        &state.config.upload_dir,
    );
    for iTopicId in vecTopicIds {
        cQueue.vUpdateMessage(*iTopicId, true).await?;
    }
    Ok(())
}

async fn reindex_topics_with_tag(state: &AppState, tag_id: i32) -> Result<()> {
    let topic_ids: Vec<i32> = sqlx::query_scalar("SELECT msgid FROM tags WHERE tagid=$1")
        .bind(tag_id)
        .fetch_all(&state.pool)
        .await?;
    vReindexTopicIds(state, &topic_ids).await
}

#[derive(Deserialize)]
pub struct TagDeleteQuery {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: String,
}

#[allow(clippy::too_many_arguments)]
fn stRenderDeleteTagForm(
    sOldTagName: &str,
    optTagName: Option<&str>,
    bCreateSynonym: bool,
    bSynonym: bool,
    sFirstLetter: &str,
    sCsrfToken: &str,
    vecErrors: &[String],
) -> Result<Html<String>> {
    let sReplacementControls = if bSynonym {
        String::new()
    } else {
        format!(
            r#"
<div class="control-group">
<label for="tagName">Метка, которой нужно заменить удаляемую (пусто - удалить без замены):</label>
<input autofocus autocapitalize="off" data-tags-autocomplete-single="data-tags-autocomplete-single" id="tagName" name="tagName" value="{tag_attr}" style="width: 40em">
</div>
<div class="control-group">
<label><input id="createSynonym" name="createSynonym" type="checkbox" value="true"{checked}> создать синоним</label>
</div>
"#,
            tag_attr = html_escape::encode_double_quoted_attribute(optTagName.unwrap_or("")),
            checked = if bCreateSynonym { " checked" } else { "" },
        )
    };
    let sCancelUrl = format!("/tags/{}", urlencoding::encode(sFirstLetter));
    let sContentHtml = format!(
        r#"
<script>$script.ready("plugins", function() {{ $script("/js/tagsAutocomplete.js"); }});</script>
<h1>Удаление метки «{old_text}»</h1>
<p><strong>Внимание!</strong> Удаление метки нельзя отменить. Изменение не отражается в истории правок топика.</p>
<form method="post" action="/tags/delete">
<input type="hidden" name="csrf" value="{csrf_token}">
{errors}
<input type="hidden" name="oldTagName" value="{old_attr}">
{replacement_controls}
<div class="form-actions">
<button type="submit" class="btn btn-danger">Удалить</button>
<button type="button" class="btn btn-default" onclick="window.location='{cancel_url}'">Отменить</button>
</div>
</form>
"#,
        old_text = html_escape::encode_text(sOldTagName),
        csrf_token = html_escape::encode_double_quoted_attribute(sCsrfToken),
        errors = sTagFormErrorsHtml(vecErrors),
        old_attr = html_escape::encode_double_quoted_attribute(sOldTagName),
        replacement_controls = sReplacementControls,
        cancel_url = html_escape::encode_double_quoted_attribute(&sCancelUrl),
    );
    Ok(Html(crate::routes::sRenderLegacyContent(
        "Удаление метки",
        sContentHtml,
    )?))
}

pub async fn delete_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<TagDeleteQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let is_synonym: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags_synonyms WHERE value=$1)")
            .bind(&q.tag_name)
            .fetch_one(&state.pool)
            .await?;
    stRenderDeleteTagForm(
        &q.tag_name,
        None,
        false,
        is_synonym,
        q.first_letter.as_deref().unwrap_or(""),
        &csrf_token,
        &[],
    )
}

#[derive(Deserialize)]
pub struct TagDeleteForm {
    #[serde(rename = "oldTagName")]
    pub old_tag_name: String,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    #[serde(rename = "createSynonym")]
    pub create_synonym: Option<String>,
}

pub async fn delete_tag(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<TagDeleteForm>,
) -> Result<Response> {
    let moderator = user
        .as_ref()
        .filter(|u| u.canmod)
        .ok_or(AppError::Forbidden)?;
    let old_tag_name = form.old_tag_name.as_str();

    // A synonym entry isn't a real tag - deleting it just drops the redirect.
    let synonym_target: Option<i32> =
        sqlx::query_scalar("SELECT tagid FROM tags_synonyms WHERE value=$1")
            .bind(old_tag_name)
            .fetch_optional(&state.pool)
            .await?;
    if synonym_target.is_some() {
        sqlx::query("DELETE FROM tags_synonyms WHERE value=$1")
            .bind(old_tag_name)
            .execute(&state.pool)
            .await?;
        tracing::info!(
            tag = %old_tag_name,
            moderator = %moderator.nick,
            "tag synonym deleted"
        );
        return Ok(crate::routes::stFoundRedirect(format!(
            "/tags/{}",
            urlencoding::encode(&first_letter_of(old_tag_name))
        )));
    }

    let optOldTagId = get_tag_id(&state.pool, old_tag_name).await?;
    let tag_name = form.tag_name.as_deref().filter(|s| !s.is_empty());
    let create_synonym = form.create_synonym.is_some();
    let vecErrors = vecDeleteTagErrors(
        optOldTagId.is_some(),
        old_tag_name,
        form.tag_name.as_deref(),
        create_synonym,
    );
    if !vecErrors.is_empty() {
        let sFirstLetter = old_tag_name.chars().next().into_iter().collect::<String>();
        return Ok(stRenderDeleteTagForm(
            old_tag_name,
            form.tag_name.as_deref(),
            create_synonym,
            false,
            &sFirstLetter,
            &csrf_token,
            &vecErrors,
        )?
        .into_response());
    }
    let old_tag_id = optOldTagId.expect("validated existing tag");

    let Some(tag_name) = tag_name else {
        let affected_topics: Vec<i32> = sqlx::query_scalar("SELECT msgid FROM tags WHERE tagid=$1")
            .bind(old_tag_id)
            .fetch_all(&state.pool)
            .await?;
        let mut tx = state.pool.begin().await?;
        sqlx::query("DELETE FROM user_tags WHERE tag_id=$1")
            .bind(old_tag_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tags WHERE tagid=$1")
            .bind(old_tag_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tags_synonyms WHERE tagid=$1")
            .bind(old_tag_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tags_values WHERE id=$1")
            .bind(old_tag_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        // TagModificationService.delete: reindex every topic that lost the tag.
        vReindexTopicIds(&state, &affected_topics).await?;
        tracing::info!(
            tag = %old_tag_name,
            moderator = %moderator.nick,
            "tag deleted"
        );
        return Ok(crate::routes::stFoundRedirect(format!(
            "/tags/{}",
            urlencoding::encode(&first_letter_of(old_tag_name))
        )));
    };

    let mut tx = state.pool.begin().await?;
    // TagService.getOrCreateTag resolves an exact canonical name first, then
    // an exact synonym, and creates a canonical tag only when neither exists.
    // In particular, merging into a synonym must target its existing tagid
    // instead of creating a second tags_values row with the synonym's name.
    let optCanonicalTagId: Option<i32> =
        sqlx::query_scalar("SELECT id FROM tags_values WHERE value=$1")
            .bind(tag_name)
            .fetch_optional(&mut *tx)
            .await?;
    let optResolvedTagId = if optCanonicalTagId.is_some() {
        optCanonicalTagId
    } else {
        sqlx::query_scalar("SELECT tagid FROM tags_synonyms WHERE value=$1")
            .bind(tag_name)
            .fetch_optional(&mut *tx)
            .await?
    };
    let new_tag_id: i32 = if let Some(iTagId) = optResolvedTagId {
        iTagId
    } else {
        sqlx::query_scalar("INSERT INTO tags_values(value) VALUES($1) RETURNING id")
            .bind(tag_name)
            .fetch_one(&mut *tx)
            .await?
    };

    // TopicTagDao.getCountReplacedTags/increaseCounterById: the original
    // increments the persisted target counter only by rows that will really
    // move. It deliberately does not replace that counter with raw count(*),
    // because the hourly recalculation excludes deleted/uncommitted topics.
    let iReplacedTagCount: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM tags old_tag
            WHERE old_tag.tagid=$1
              AND NOT EXISTS (
                SELECT 1 FROM tags target_tag
                 WHERE target_tag.msgid=old_tag.msgid AND target_tag.tagid=$2
              )"#,
    )
    .bind(old_tag_id)
    .bind(new_tag_id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO tags(msgid,tagid) SELECT msgid,$2 FROM tags WHERE tagid=$1 ON CONFLICT DO NOTHING")
        .bind(old_tag_id).bind(new_tag_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM tags WHERE tagid=$1")
        .bind(old_tag_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_tags(user_id,tag_id,is_favorite) SELECT user_id,$2,is_favorite FROM user_tags WHERE tag_id=$1 ON CONFLICT DO NOTHING")
        .bind(old_tag_id).bind(new_tag_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM user_tags WHERE tag_id=$1")
        .bind(old_tag_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE tags_values SET counter=counter+$2 WHERE id=$1")
        .bind(new_tag_id)
        .bind(iReplacedTagCount)
        .execute(&mut *tx)
        .await?;

    // Any synonym that pointed at the tag being removed now follows the merge target.
    sqlx::query("UPDATE tags_synonyms SET tagid=$2 WHERE tagid=$1")
        .bind(old_tag_id)
        .bind(new_tag_id)
        .execute(&mut *tx)
        .await?;
    if create_synonym {
        sqlx::query("INSERT INTO tags_synonyms(value,tagid) VALUES($1,$2) ON CONFLICT(value) DO UPDATE SET tagid=EXCLUDED.tagid")
            .bind(old_tag_name).bind(new_tag_id).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM tags_values WHERE id=$1")
        .bind(old_tag_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // TagModificationService.merge: reindex every topic now carrying the merge target's tag.
    reindex_topics_with_tag(&state, new_tag_id).await?;
    tracing::info!(
        old_tag = %old_tag_name,
        new_tag = %tag_name,
        create_synonym,
        moderator = %moderator.nick,
        "tag merged"
    );

    Ok(crate::routes::stFoundRedirect(format!(
        "/tags/{}",
        urlencoding::encode(&first_letter_of(tag_name))
    )))
}

fn vecDeleteTagErrors(
    bOldTagExists: bool,
    sOldTagName: &str,
    optTagName: Option<&str>,
    bCreateSynonym: bool,
) -> Vec<String> {
    let mut vecErrors = Vec::new();
    let bPerformDelete = optTagName.is_none_or(str::is_empty);
    if !bOldTagExists {
        vecErrors.push("Тега с таким именем не существует!".into());
    }
    if let Some(sTagName) = optTagName.filter(|sTagName| !sTagName.is_empty())
        && !is_good_tag(sTagName)
    {
        vecErrors.push(format!("Некорректный тег: '{sTagName}'"));
    }
    if optTagName.is_some_and(|sTagName| sOldTagName == sTagName) {
        vecErrors.push("Заменяемый тег не должен быть равен удаляемому!".into());
    }
    if bCreateSynonym && bPerformDelete {
        vecErrors.push("Не указан тег для создания синонима!".into());
    }
    vecErrors
}

#[cfg(test)]
mod tag_mutation_form_validation_tests {
    use super::*;

    #[test]
    fn change_validation_accumulates_errors_in_java_order() {
        let vecErrors = vecChangeTagErrors(false, false, " invalid");
        assert_eq!(
            vecErrors,
            vec![
                "Тега с таким именем не существует!",
                "Некорректный тег: ' invalid'",
            ]
        );

        assert_eq!(
            vecChangeTagErrors(true, true, "existing"),
            vec!["Тег с таким именем уже существует!"]
        );
    }

    #[test]
    fn delete_validation_accumulates_errors_without_trimming_values() {
        let vecErrors = vecDeleteTagErrors(false, "?bad", Some("?bad"), false);
        assert_eq!(
            vecErrors,
            vec![
                "Тега с таким именем не существует!",
                "Некорректный тег: '?bad'",
                "Заменяемый тег не должен быть равен удаляемому!",
            ]
        );
        assert_eq!(
            vecDeleteTagErrors(true, "old", Some(""), true),
            vec!["Не указан тег для создания синонима!"]
        );
    }

    #[test]
    fn merge_counter_increments_only_rows_really_moved() {
        let sSource = include_str!("tags.rs");
        let sMerge = sSource
            .split_once("let iReplacedTagCount: i64")
            .expect("merge count")
            .1
            .split_once("// Any synonym")
            .expect("end of merge counter block")
            .0;
        assert!(sMerge.contains("NOT EXISTS"));
        assert!(sMerge.contains("target_tag.msgid=old_tag.msgid"));
        assert!(sMerge.contains("counter=counter+$2"));
        assert!(sMerge.contains(".bind(iReplacedTagCount)"));
        assert!(!sMerge.contains("SET counter=(SELECT count(*)"));
    }

    #[test]
    fn merge_resolves_exact_canonical_then_synonym_before_creating_tag() {
        let sSource = include_str!("tags.rs");
        let sResolution = sSource
            .split_once("// TagService.getOrCreateTag resolves")
            .expect("merge target resolution")
            .1
            .split_once("// TopicTagDao.getCountReplacedTags")
            .expect("end of merge target resolution")
            .0;
        let iCanonical = sResolution
            .find("SELECT id FROM tags_values WHERE value=$1")
            .expect("exact canonical lookup");
        let iSynonym = sResolution
            .find("SELECT tagid FROM tags_synonyms WHERE value=$1")
            .expect("exact synonym lookup");
        let iCreate = sResolution
            .find("INSERT INTO tags_values(value) VALUES($1) RETURNING id")
            .expect("fallback tag creation");

        assert!(iCanonical < iSynonym && iSynonym < iCreate);
        assert!(sResolution.contains("if optCanonicalTagId.is_some()"));
        assert!(sResolution.contains("if let Some(iTagId) = optResolvedTagId"));
        assert!(!sResolution.contains("ON CONFLICT"));
    }

    #[test]
    fn invalid_forms_retain_values_errors_and_theme_shell() {
        let Html(sChangeHtml) = stRenderChangeTagForm(
            "old",
            "bad<name",
            "b",
            "csrf-token",
            &["Некорректный тег: 'bad<name'".into()],
        )
        .expect("change form");
        assert_eq!(
            Html(sChangeHtml.clone()).into_response().status(),
            StatusCode::OK
        );
        assert!(sChangeHtml.contains("<main id=\"bd\">"));
        assert!(sChangeHtml.contains("class=\"error\""));
        assert!(sChangeHtml.contains("name=\"tagName\""));
        assert!(sChangeHtml.contains("bad&lt;name"));
        assert!(!sChangeHtml.contains("value=\"bad<name\""));

        let Html(sDeleteHtml) = stRenderDeleteTagForm(
            "old",
            Some("replacement"),
            true,
            false,
            "o",
            "csrf-token",
            &["Ошибка".into()],
        )
        .expect("delete form");
        assert!(sDeleteHtml.contains("<main id=\"bd\">"));
        assert!(sDeleteHtml.contains("class=\"error\">Ошибка</div>"));
        assert!(sDeleteHtml.contains("value=\"replacement\""));
        assert!(sDeleteHtml.contains("id=\"createSynonym\""));
        assert!(sDeleteHtml.contains(" checked"));
    }

    #[test]
    fn synonym_delete_form_hides_merge_controls_like_java_jsp() {
        let Html(sHtml) = stRenderDeleteTagForm("alias", None, false, true, "a", "csrf-token", &[])
            .expect("synonym delete form");
        assert!(!sHtml.contains("id=\"tagName\""));
        assert!(!sHtml.contains("id=\"createSynonym\""));
    }
}

#[cfg(test)]
mod tag_section_binding_tests {
    use super::*;

    #[test]
    fn malformed_numeric_parameters_use_the_live_spring_400_contract() {
        assert_eq!(iTagSectionParameter("", "section").unwrap(), 0);
        assert_eq!(iTagSectionParameter("  -1  ", "offset").unwrap(), -1);

        for sName in ["section", "offset"] {
            let stError = iTagSectionParameter("invalid", sName).unwrap_err();
            assert!(matches!(&stError, AppError::BadRequest(_)));
            assert_eq!(stError.into_response().status(), StatusCode::BAD_REQUEST);
        }
    }
}
