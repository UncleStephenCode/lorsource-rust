use crate::{
    application::topic::{
        CTopicService,
        edit::{CTopicEditService, EnTopicEditOutcome, StPreparedTopicEdit, StTopicEditInput},
        posting::CAddTopicService,
    },
    auth::CurrentUser,
    domain::topic::{
        edit::{
            StTopicEditActor, StTopicEditPoll, StTopicEditPollValue, TrTopicEditRealtimeNotifier,
        },
        posting::{StAddTopicActor, StAddTopicPermission, StTopicLimitInfo},
        repository::StNewTopic,
    },
    error::{AppError, Result},
    infra::postgres::{
        add_topic_repository::CAddTopicPgRepository, topic_edit_repository::CTopicEditPgRepository,
        topic_repository::CTopicPgRepository,
    },
    infra::search_queue::CSearchQueueSender,
    markup,
    models::{CommentItem, Group, PagerQuery, TopicDetail, TopicSummary, UserSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, Uri, header, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{MethodRouter, get},
};
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    news: Vec<NewsTopicView>,
    main_page: bool,
    tracker_layout: bool,
    navigation: Option<TopicListNavigation>,
    prev_link: Option<String>,
    next_link: Option<String>,
}

#[derive(Template)]
#[template(path = "main_page.html")]
struct MainPageTemplate {
    news: Vec<NewsTopicView>,
    brief: Vec<TopicSummary>,
    add_url: Option<String>,
    add_reason: String,
    uncommitted: Vec<(i32, String, i64)>,
    current_user: Option<UserSummary>,
    user_status: String,
    drafts_count: i64,
    favorite_present: bool,
    boxlets_html: String,
    show_gallery_on_main: bool,
}

#[derive(Template)]
#[template(path = "main_boxlets.html")]
struct StMainBoxletsTemplate {
    vecBoxlets: Vec<String>,
}

const ARR_MAIN_MIXED_BOXLETS: [&str; 2] = ["top10", "tagcloud"];
const ARR_MAIN_NEWS_BOXLETS: [&str; 5] = ["poll", "articles", "top10", "gallery", "tagcloud"];

fn arrMainBoxlets(bShowGalleryOnMain: bool) -> &'static [&'static str] {
    if bShowGalleryOnMain {
        &ARR_MAIN_MIXED_BOXLETS
    } else {
        &ARR_MAIN_NEWS_BOXLETS
    }
}

/// MainPageController keeps appending full cards until it has seen ten
/// non-minor topics.  A minor topic before that boundary is a full card too,
/// but does not advance the counter.
fn bMainTopicUsesFullCard(bMinor: bool, iFullNonMinor: &mut usize) -> bool {
    let bFullCard = *iFullNonMinor < 10;
    if bFullCard && !bMinor {
        *iFullNonMinor += 1;
    }
    bFullCard
}

async fn vecRenderMainBoxlets(
    stState: &AppState,
    bShowGalleryOnMain: bool,
    iMessagesPerPage: i32,
    optUserId: Option<i32>,
    sCsrfToken: &str,
) -> Result<Vec<String>> {
    let mut vecBoxlets = Vec::with_capacity(arrMainBoxlets(bShowGalleryOnMain).len());
    for sBoxletName in arrMainBoxlets(bShowGalleryOnMain) {
        let sBoxlet = match *sBoxletName {
            "poll" => {
                crate::routes::api::sRenderPollBoxlet(
                    stState,
                    optUserId,
                    optUserId.is_some(),
                    sCsrfToken.to_owned(),
                )
                .await?
            }
            "articles" => {
                crate::routes::api::sRenderArticlesBoxlet(stState, iMessagesPerPage).await?
            }
            "top10" => crate::routes::api::sRenderTop10Boxlet(stState, iMessagesPerPage).await?,
            "gallery" => crate::routes::boxlets::sRenderGalleryBoxlet(stState).await?,
            "tagcloud" => crate::routes::boxlets::sRenderTagCloudBoxlet(stState).await?,
            _ => unreachable!("Profile.getBoxlets contains an unknown boxlet"),
        };
        vecBoxlets.push(sBoxlet);
    }
    Ok(vecBoxlets)
}

#[cfg(test)]
mod main_boxlet_layout_tests {
    use askama::Template;

    use super::{StMainBoxletsTemplate, arrMainBoxlets, bMainTopicUsesFullCard};

    #[test]
    fn mixed_main_profile_uses_exact_java_boxlet_order() {
        assert_eq!(arrMainBoxlets(true), ["top10", "tagcloud"]);
    }

    #[test]
    fn news_only_profile_uses_exact_java_boxlet_order() {
        assert_eq!(
            arrMainBoxlets(false),
            ["poll", "articles", "top10", "gallery", "tagcloud"]
        );
    }

    #[test]
    fn main_wraps_direct_fragments_once_and_keeps_their_order() {
        let sHtml = StMainBoxletsTemplate {
            vecBoxlets: vec![
                "<h2>poll</h2><div class=\"boxlet_content\">P</div>".to_owned(),
                "<h2>articles</h2><div class=\"boxlet_content\">A</div>".to_owned(),
            ],
        }
        .render()
        .expect("main boxlets partial");

        assert_eq!(sHtml.matches("<div class=\"boxlet\">").count(), 2);
        assert!(sHtml.find("<h2>poll</h2>") < sHtml.find("<h2>articles</h2>"));
        assert!(!sHtml.contains("&lt;h2&gt;"));
    }

    #[test]
    fn main_template_has_no_legacy_duplicate_boxlet_markup() {
        let sTemplate = include_str!("../../templates/main_page.html");
        assert!(sTemplate.contains("{{ boxlets_html|safe }}"));
        for sLegacyField in ["top_topics", "gallery.len()", "tags.len()", "match poll"] {
            assert!(!sTemplate.contains(sLegacyField));
        }
    }

    #[test]
    fn minor_topic_before_tenth_regular_topic_remains_a_full_card() {
        let vecMinor = [
            false, false, false, false, false, false, false, false, false, true, false, true,
        ];
        let mut iFullNonMinor = 0;
        let vecFull = vecMinor
            .into_iter()
            .map(|bMinor| bMainTopicUsesFullCard(bMinor, &mut iFullNonMinor))
            .collect::<Vec<_>>();

        assert_eq!(
            vecFull,
            [true; 11].into_iter().chain([false]).collect::<Vec<_>>()
        );
        assert_eq!(iFullNonMinor, 10);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QuickGroupLink {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveTagLink {
    pub(crate) name: String,
    pub(crate) url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ForumFilterLink {
    pub(crate) label: &'static str,
    pub(crate) url: &'static str,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TopicListNavigation {
    pub(crate) section_id: i32,
    pub(crate) section_url: Option<String>,
    pub(crate) archive_url: Option<String>,
    pub(crate) rss_url: Option<String>,
    pub(crate) add_url: Option<String>,
    pub(crate) add_reason: String,
    pub(crate) moderator_group_id: Option<i32>,
    pub(crate) quick_groups: Vec<QuickGroupLink>,
    pub(crate) all_groups_selected: bool,
    pub(crate) uncommitted_count: i64,
    pub(crate) active_tags: Vec<ActiveTagLink>,
    pub(crate) forum_filters: Vec<ForumFilterLink>,
}

#[derive(Debug, Clone)]
struct CommentPageLink {
    page: i64,
    current: bool,
}

#[derive(Template)]
#[template(path = "topic.html")]
struct TopicTemplate {
    topic: TopicDetail,
    canonical_url: String,
    og_image_url: String,
    topic_card_html: String,
    comments: Vec<CommentView>,
    /// Non-empty only outside thread/deleted mode, when there's more than
    /// one page of comments (TopicController.buildPages).
    pages: Vec<CommentPageLink>,
    thread_root: Option<i32>,
    show_deleted: bool,
    /// Java's `showDeletedButton`: only a moderator viewing the live
    /// (non-deleted-mode) page gets offered the toggle.
    show_deleted_button: bool,
    /// Comments hidden by the viewer's ignore list in the current filtered
    /// view (TopicController's hideSet) - `unfiltered_count` is Java's
    /// `unfilteredCount`, used to render "N скрыто, показать".
    filtered_count: usize,
    unfiltered_count: usize,
    csrf_token: String,
    comment_format_mode: String,
    comment_format_title: String,
    can_comment: bool,
    anonymous_comment_form: bool,
    require_comment_captcha: bool,
    captcha_site_key: String,
    realtime_bootstrap_html: String,
    related_topics: Vec<Vec<crate::search_index::StSimilarTopic>>,
}

#[derive(Template)]
#[template(path = "topic_card.html")]
struct StTopicCardTemplate {
    card: StTopicCardView,
}

struct StTopicCardView {
    topic: TopicDetail,
    title_plain: String,
    topic_author_signature: AuthorSignatureView,
    topic_html: String,
    poll: Option<PollView>,
    images_html: String,
    topic_reactions_html: String,
    topic_show_reactions_link: bool,
    show_menu: bool,
    enable_schema: bool,
    links_allowed: bool,
    topic_expired: bool,
    resolved: bool,
    moderator_menu: bool,
    can_commit: bool,
    can_comment: bool,
    can_edit: bool,
    can_delete: bool,
    can_resolve: bool,
    can_warn: bool,
    show_postscore: bool,
    postscore_info_html: String,
    deleted_header_html: String,
    userpic_html: String,
    author_html: String,
    remark_html: String,
    moderator_ip_html: String,
    committer_html: String,
    edit_summary_html: String,
    moderator_user_agent_html: String,
    warnings_html: String,
    memories_buttons_html: String,
    memories_script_html: String,
}

#[derive(sqlx::FromRow)]
struct StTopicCardMeta {
    #[sqlx(rename = "bLinksAllowed")]
    bLinksAllowed: bool,
    #[sqlx(rename = "bResolvable")]
    bResolvable: bool,
    #[sqlx(rename = "bExpired")]
    bExpired: bool,
    #[sqlx(rename = "iTopicPostScore")]
    iTopicPostScore: i32,
    #[sqlx(rename = "iRestrictComments")]
    iRestrictComments: i32,
    #[sqlx(rename = "iCommentCount")]
    iCommentCount: i32,
    #[sqlx(rename = "iOpenWarnings")]
    iOpenWarnings: i32,
    #[sqlx(rename = "bAllowAnonymous")]
    bAllowAnonymous: bool,
    #[sqlx(rename = "iScoreLoss")]
    iScoreLoss: i32,
    #[sqlx(rename = "optCommitDate")]
    optCommitDate: Option<chrono::DateTime<chrono::Utc>>,
    #[sqlx(rename = "optCommitterNick")]
    optCommitterNick: Option<String>,
    #[sqlx(rename = "bCommitterBlocked")]
    bCommitterBlocked: bool,
    #[sqlx(rename = "sPostIp")]
    sPostIp: String,
    #[sqlx(rename = "iUserAgentId")]
    iUserAgentId: i32,
    #[sqlx(rename = "optUserAgent")]
    optUserAgent: Option<String>,
    #[sqlx(rename = "optRemark")]
    optRemark: Option<String>,
    #[sqlx(rename = "optDeleteUserNick")]
    optDeleteUserNick: Option<String>,
    #[sqlx(rename = "optDeleteReason")]
    optDeleteReason: Option<String>,
}

struct StTopicCardBuildInput {
    topic: TopicDetail,
    title_plain: String,
    topic_author_signature: AuthorSignatureView,
    topic_html: String,
    poll: Option<PollView>,
    images_html: String,
    topic_reactions: ReactionsWidget,
    userpic_html: String,
    can_comment: bool,
    actor_frozen: bool,
    show_menu: bool,
    enable_schema: bool,
    include_canonical_extras: bool,
    remote_ip: String,
}

fn iSectionCommentPostScore(iSectionId: i32) -> i32 {
    match iSectionId {
        1 | 2 => crate::domain::topic::options::POSTSCORE_UNRESTRICTED,
        3 | 5 | 6 => 45,
        _ => 50,
    }
}

fn iEffectiveTopicPostScore(stTopic: &TopicDetail, stMeta: &StTopicCardMeta) -> i32 {
    let iCommentCountRestriction = if stTopic.sticky {
        crate::domain::topic::options::POSTSCORE_UNRESTRICTED
    } else if stMeta.iCommentCount > 3000 {
        200
    } else if stMeta.iCommentCount > 2000 {
        100
    } else if stMeta.iCommentCount > 1000 {
        50
    } else {
        crate::domain::topic::options::POSTSCORE_UNRESTRICTED
    };
    let iScoreLossRestriction = if stTopic.sticky || stMeta.bExpired {
        crate::domain::topic::options::POSTSCORE_UNRESTRICTED
    } else if stMeta.iScoreLoss >= 150 {
        100
    } else if stMeta.iScoreLoss >= 100 {
        50
    } else {
        crate::domain::topic::options::POSTSCORE_UNRESTRICTED
    };
    [
        stMeta.iTopicPostScore,
        stMeta.iRestrictComments,
        iSectionCommentPostScore(stTopic.section_id),
        iCommentCountRestriction,
        if stMeta.bAllowAnonymous {
            crate::domain::topic::options::POSTSCORE_UNRESTRICTED
        } else {
            crate::domain::topic::options::POSTSCORE_REGISTERED_ONLY
        },
        iScoreLossRestriction,
        if stMeta.iOpenWarnings > 2 {
            100
        } else {
            crate::domain::topic::options::POSTSCORE_UNRESTRICTED
        },
    ]
    .into_iter()
    .max()
    .unwrap_or(crate::domain::topic::options::POSTSCORE_UNRESTRICTED)
}

fn sTopicCardUserHtml(sNick: &str, bBlocked: bool, bLink: bool, sAttributes: &str) -> String {
    let sNickText = html_escape::encode_text(sNick);
    let sBody = if bLink {
        format!(
            "<a{sAttributes} href=\"/people/{}/profile\">{sNickText}</a>",
            urlencoding::encode(sNick)
        )
    } else {
        sNickText.into_owned()
    };
    if bBlocked {
        format!("<s>{sBody}</s>")
    } else {
        sBody
    }
}

async fn stTopicCardMeta(
    stState: &AppState,
    iTopicId: i32,
    optViewerId: Option<i32>,
) -> Result<StTopicCardMeta> {
    sqlx::query_as(
        r#"SELECT s.havelink AS "bLinksAllowed", g.resolvable AS "bResolvable",
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire AS "bExpired",
                  COALESCE(t.postscore,-9999) AS "iTopicPostScore",
                  g.restrict_comments AS "iRestrictComments", t.stat1 AS "iCommentCount",
                  t.open_warnings AS "iOpenWarnings", t.allow_anonymous AS "bAllowAnonymous",
                  COALESCE((SELECT sum(-di.bonus) FROM del_info di
                            JOIN comments dc ON dc.id=di.msgid
                            WHERE di.bonus IS NOT NULL AND di.bonus<>0
                              AND dc.userid<>2 AND dc.deleted AND dc.topic=t.id),0)::int AS "iScoreLoss",
                  t.commitdate AS "optCommitDate", committer.nick AS "optCommitterNick",
                  COALESCE(committer.blocked,false) AS "bCommitterBlocked",
                  COALESCE(host(t.postip),'') AS "sPostIp", COALESCE(t.ua_id,0) AS "iUserAgentId",
                  ua.name AS "optUserAgent", remark.remark_text AS "optRemark",
                  delete_user.nick AS "optDeleteUserNick", di.reason AS "optDeleteReason"
             FROM topics t
             JOIN groups g ON g.id=t.groupid
             JOIN sections s ON s.id=g.section
             LEFT JOIN users committer ON committer.id=t.commitby
             LEFT JOIN user_agents ua ON ua.id=t.ua_id
             LEFT JOIN user_remarks remark ON remark.user_id=$2 AND remark.ref_user_id=t.userid
             LEFT JOIN del_info di ON di.msgid=t.id
             LEFT JOIN users delete_user ON delete_user.id=di.delby
            WHERE t.id=$1"#,
    )
    .bind(iTopicId)
    .bind(optViewerId)
    .fetch_optional(&stState.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn sTopicEditSummaryHtml(
    stState: &AppState,
    stTopic: &TopicDetail,
    optUser: &Option<UserSummary>,
    bExpired: bool,
) -> Result<String> {
    let optRow: Option<(String, chrono::DateTime<chrono::Utc>, i64)> = sqlx::query_as(
        r#"SELECT u.nick,e.editdate,count(*) OVER()::bigint
             FROM edit_info e JOIN users u ON u.id=e.editor
            WHERE e.msgid=$1 AND e.object_type='TOPIC'::edit_event_type
            ORDER BY e.id DESC LIMIT 1"#,
    )
    .bind(stTopic.id)
    .fetch_optional(&stState.pool)
    .await?;
    let Some((sEditor, dtEdit, iCount)) = optRow else {
        return Ok(String::new());
    };
    let bShowHistory = optUser
        .as_ref()
        .is_some_and(|stUser| stUser.canmod || stUser.id == stTopic.author_id || !bExpired);
    let sCount = if bShowHistory {
        format!(
            "(\u{0432}\u{0441}\u{0435}\u{0433}\u{043e} <a href=\"{}/history\">\u{0438}\u{0441}\u{043f}\u{0440}\u{0430}\u{0432}\u{043b}\u{0435}\u{043d}\u{0438}\u{0439}: {iCount}</a>)",
            stTopic.topic_url()
        )
    } else {
        format!(
            "(\u{0432}\u{0441}\u{0435}\u{0433}\u{043e} \u{0438}\u{0441}\u{043f}\u{0440}\u{0430}\u{0432}\u{043b}\u{0435}\u{043d}\u{0438}\u{0439}: {iCount})"
        )
    };
    Ok(format!(
        "<br>\u{041f}\u{043e}\u{0441}\u{043b}\u{0435}\u{0434}\u{043d}\u{0435}\u{0435} \u{0438}\u{0441}\u{043f}\u{0440}\u{0430}\u{0432}\u{043b}\u{0435}\u{043d}\u{0438}\u{0435}: {} <time data-format=\"default\" datetime=\"{}\">{dtEdit}</time> {sCount}",
        html_escape::encode_text(&sEditor),
        dtEdit.to_rfc3339()
    ))
}

async fn sTopicWarningsHtml(
    stState: &AppState,
    iTopicId: i32,
    stUser: &UserSummary,
    sCsrfToken: &str,
    iOpenWarnings: i32,
) -> Result<String> {
    type TyWarningRow = (
        i32,
        chrono::DateTime<chrono::Utc>,
        String,
        bool,
        String,
        String,
        Option<String>,
        Option<bool>,
    );
    let vecRows: Vec<TyWarningRow> = sqlx::query_as(
        r#"SELECT w.id,w.postdate,author.nick,COALESCE(author.blocked,false),
                  w.warning_type::text,w.message,closed.nick,closed.blocked
             FROM message_warnings w
             JOIN users author ON author.id=w.author
             LEFT JOIN users closed ON closed.id=w.closed_by
            WHERE w.topic=$1 AND w.comment IS NULL
              AND ($2 OR w.warning_type IN ('tag','spelling'))
            ORDER BY w.postdate"#,
    )
    .bind(iTopicId)
    .bind(stUser.canmod)
    .fetch_all(&stState.pool)
    .await?;
    if vecRows.is_empty() {
        return Ok(String::new());
    }
    let mut sHtml = String::from("<div class=\"infoblock\">");
    for (iId, dtPost, sAuthor, bAuthorBlocked, sType, sMessage, optClosed, optClosedBlocked) in
        vecRows
    {
        let sTypeName = crate::domain::warning::model::EnWarningType::optFromId(&sType)
            .map(|enType| enType.sName())
            .unwrap_or(&sType);
        let sAuthorHtml = sTopicCardUserHtml(&sAuthor, bAuthorBlocked, true, "");
        sHtml.push_str("<div style=\"margin-bottom: 0.5em\">⚠️ ");
        if optClosed.is_some() {
            sHtml.push_str("<s>");
        }
        sHtml.push_str(&format!(
            "<time data-format=\"default\" datetime=\"{}\">{dtPost}</time> {sAuthorHtml}: [{}] {}",
            dtPost.to_rfc3339(),
            html_escape::encode_text(sTypeName),
            html_escape::encode_text(&sMessage)
        ));
        if let Some(sClosed) = optClosed {
            sHtml.push_str(&format!(
                "</s> (\u{0437}\u{0430}\u{043a}\u{0440}\u{044b}\u{0442} {})",
                sTopicCardUserHtml(&sClosed, optClosedBlocked.unwrap_or(false), true, "")
            ));
        } else {
            sHtml.push_str(&format!(
                "&nbsp;<form class=\"clear-warning-form\" action=\"clear-warning\" method=\"POST\" style=\"display: inline-block\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"id\" value=\"{iId}\"><button type=\"submit\" class=\"btn btn-small btn-default\">\u{0437}\u{0430}\u{043a}\u{0440}\u{044b}\u{0442}\u{044c}</button></form>",
                html_escape::encode_double_quoted_attribute(sCsrfToken)
            ));
        }
        sHtml.push_str("</div>");
    }
    if iOpenWarnings > 2 && stUser.canmod {
        sHtml.push_str("<div style=\"margin-bottom: 0.5em\">⚠️ \u{041f}\u{0440}\u{0435}\u{0432}\u{044b}\u{0448}\u{0435}\u{043d}\u{043e} \u{0447}\u{0438}\u{0441}\u{043b}\u{043e} \u{043f}\u{0440}\u{0435}\u{0434}\u{0443}\u{043f}\u{0440}\u{0435}\u{0436}\u{0434}\u{0435}\u{043d}\u{0438}\u{0439}. \u{0421}\u{043e}\u{043e}\u{0431}\u{0449}\u{0435}\u{043d}\u{0438}\u{0435} \u{0441}\u{043a}\u{0440}\u{044b}\u{0442}\u{043e} \u{0434}\u{043b}\u{044f} \u{043d}\u{0435}\u{0430}\u{0432}\u{0442}\u{043e}\u{0440}\u{0438}\u{0437}\u{043e}\u{0432}\u{0430}\u{043d}\u{043d}\u{044b}\u{0445} \u{043f}\u{043e}\u{0441}\u{0435}\u{0442}\u{0438}\u{0442}\u{0435}\u{043b}\u{0435}\u{0439}.</div>");
    }
    sHtml.push_str("</div>");
    Ok(sHtml)
}

async fn stTopicMemoriesHtml(
    stState: &AppState,
    iTopicId: i32,
    optUser: &Option<UserSummary>,
    sCsrfToken: &str,
) -> Result<(String, String)> {
    let vecCounts: Vec<(bool, i64)> =
        sqlx::query_as("SELECT watch,count(*)::bigint FROM memories WHERE topic=$1 GROUP BY watch")
            .bind(iTopicId)
            .fetch_all(&stState.pool)
            .await?;
    let mut iWatchCount = 0;
    let mut iFavCount = 0;
    for (bWatch, iCount) in vecCounts {
        if bWatch {
            iWatchCount = iCount;
        } else {
            iFavCount = iCount;
        }
    }
    let mut iWatchId = 0;
    let mut iFavId = 0;
    if let Some(stUser) = optUser {
        for (iId, bWatch) in sqlx::query_as::<_, (i32, bool)>(
            "SELECT id,watch FROM memories WHERE userid=$1 AND topic=$2",
        )
        .bind(stUser.id)
        .bind(iTopicId)
        .fetch_all(&stState.pool)
        .await?
        {
            if bWatch {
                iWatchId = iId;
            } else {
                iFavId = iId;
            }
        }
    }
    let sButtons = format!(
        "<div class=\"fav-buttons\"><div><a id=\"favs_button\" href=\"#\"{} title=\"{}\"><i class=\"icon-star\"></i></a><br><span id=\"favs_count\">{iFavCount}</span><br></div><div><a id=\"memories_button\" href=\"#\"{} title=\"{}\"><i class=\"icon-bell\"></i></a><br><span id=\"memories_count\">{iWatchCount}</span></div></div>",
        if iFavId != 0 {
            " class=\"selected\""
        } else {
            ""
        },
        if iFavId != 0 {
            "\u{0423}\u{0434}\u{0430}\u{043b}\u{0438}\u{0442}\u{044c} \u{0438}\u{0437} \u{0438}\u{0437}\u{0431}\u{0440}\u{0430}\u{043d}\u{043d}\u{043e}\u{0433}\u{043e}"
        } else {
            "\u{0412} \u{0438}\u{0437}\u{0431}\u{0440}\u{0430}\u{043d}\u{043d}\u{043e}\u{0435}"
        },
        if iWatchId != 0 {
            " class=\"selected\""
        } else {
            ""
        },
        if iWatchId != 0 {
            "\u{041d}\u{0435} \u{043e}\u{0442}\u{0441}\u{043b}\u{0435}\u{0436}\u{0438}\u{0432}\u{0430}\u{0442}\u{044c}"
        } else {
            "\u{041e}\u{0442}\u{0441}\u{043b}\u{0435}\u{0436}\u{0438}\u{0432}\u{0430}\u{0442}\u{044c}"
        },
    );
    let sScript = if optUser.is_some() {
        format!(
            "<script type=\"text/javascript\">$script.ready('lorjs', function () {{ topic_memories_form_setup({iWatchId}, true, {iTopicId}, \"{}\"); topic_memories_form_setup({iFavId}, false, {iTopicId}, \"{}\"); }});</script>",
            html_escape::encode_double_quoted_attribute(sCsrfToken),
            html_escape::encode_double_quoted_attribute(sCsrfToken)
        )
    } else {
        "<script type=\"text/javascript\">$script.ready('lorjs', function() { initStarPopovers(); });</script>".to_owned()
    };
    Ok((sButtons, sScript))
}

async fn sBuildTopicCardHtml(
    stState: &AppState,
    optUser: &Option<UserSummary>,
    sCsrfToken: &str,
    stInput: StTopicCardBuildInput,
) -> Result<String> {
    let stTopic = &stInput.topic;
    let stMeta = stTopicCardMeta(
        stState,
        stTopic.id,
        optUser.as_ref().map(|stUser| stUser.id),
    )
    .await?;
    let bModerator = optUser.as_ref().is_some_and(|stUser| stUser.canmod);
    let bAuthorized = optUser.is_some();
    let sAuthorHtml = sTopicCardUserHtml(
        &stTopic.author,
        stTopic.author_blocked,
        !stTopic.author_anonymous || bAuthorized,
        " rel=\"author\" itemprop=\"creator\"",
    );
    let sRemarkHtml = stMeta
        .optRemark
        .as_ref()
        .map_or_else(String::new, |sRemark| {
            format!(
                "&emsp;<span class=\"user-remark\">{}</span>",
                html_escape::encode_text(sRemark)
            )
        });
    let sModeratorIpHtml = if bModerator && !stMeta.sPostIp.is_empty() {
        let sIp = html_escape::encode_double_quoted_attribute(&stMeta.sPostIp);
        format!(" (<a href=\"sameip.jsp?ip={sIp}\">{sIp}</a>)")
    } else {
        String::new()
    };
    let sCommitterHtml = if stTopic.section_premoderated && stTopic.moderate {
        match stMeta.optCommitterNick.as_deref() {
            Some(sNick) if sNick != stTopic.author => {
                let mut sHtml = format!(
                    "<br>\u{041f}\u{0440}\u{043e}\u{0432}\u{0435}\u{0440}\u{0435}\u{043d}\u{043e}: {}",
                    sTopicCardUserHtml(sNick, stMeta.bCommitterBlocked, true, "")
                );
                if let Some(dtCommit) = stMeta.optCommitDate
                    && dtCommit != stTopic.postdate
                {
                    sHtml.push_str(&format!(
                        " (<time data-format=\"default\" datetime=\"{}\" itemprop=\"datePublished\">{dtCommit}</time>)",
                        dtCommit.to_rfc3339()
                    ));
                }
                sHtml
            }
            _ => String::new(),
        }
    } else {
        String::new()
    };
    let sModeratorUserAgentHtml = if bModerator {
        stMeta
            .optUserAgent
            .as_ref()
            .map_or_else(String::new, |sUserAgent| {
                format!(
                    "<br>{}&nbsp;<a href=\"sameip.jsp?ua={}&amp;ip={}&amp;mask=0\">🔍</a>",
                    html_escape::encode_text(sUserAgent),
                    stMeta.iUserAgentId,
                    html_escape::encode_double_quoted_attribute(&stMeta.sPostIp)
                )
            })
    } else {
        String::new()
    };

    let (mut bCanCommit, mut bCanEdit) = (false, false);
    if stInput.show_menu
        && let Some(stUser) = optUser
    {
        let stPrepared = cTopicEditService(stState)
            .stPrepare(stTopic.id, stTopicEditActor(stUser), &stInput.remote_ip)
            .await?;
        bCanCommit =
            stPrepared.stSnapshot.bCommittable() && stPrepared.stCommitPermission.bPermitted();
        bCanEdit = stPrepared.bAnythingEditable();
    }

    let stDeleteMeta = if stInput.show_menu {
        Some(load_topic_delete_meta(stState, stTopic.id).await?)
    } else {
        None
    };
    let bCanDelete = match (optUser.as_ref(), stDeleteMeta.as_ref()) {
        (Some(stUser), Some(stDeleteMeta)) if !stTopic.deleted => {
            b_topic_deletable(stDeleteMeta, stUser, chrono::Utc::now())
        }
        _ => false,
    };
    let bCanUndelete = match (optUser.as_ref(), stDeleteMeta.as_ref()) {
        (Some(stUser), Some(stDeleteMeta)) if stTopic.deleted => {
            bTopicUndeletable(stState, stDeleteMeta, stUser, stTopic.id).await?
        }
        _ => false,
    };
    let sDeletedHeaderHtml = if stInput.show_menu && stTopic.deleted {
        let sDescription = match (
            stMeta.optDeleteUserNick.as_deref(),
            stMeta.optDeleteReason.as_deref(),
        ) {
            (Some(sNick), Some(sReason)) => format!(
                "<strong>\u{0421}\u{043e}\u{043e}\u{0431}\u{0449}\u{0435}\u{043d}\u{0438}\u{0435} \u{0443}\u{0434}\u{0430}\u{043b}\u{0435}\u{043d}\u{043e} {} \u{043f}\u{043e} \u{043f}\u{0440}\u{0438}\u{0447}\u{0438}\u{043d}\u{0435}: '{}'</strong>",
                html_escape::encode_text(sNick),
                html_escape::encode_text(sReason)
            ),
            _ => "<strong>\u{0421}\u{043e}\u{043e}\u{0431}\u{0449}\u{0435}\u{043d}\u{0438}\u{0435} \u{0443}\u{0434}\u{0430}\u{043b}\u{0435}\u{043d}\u{043e}</strong>".to_owned(),
        };
        format!(
            "<div class=\"title\">{sDescription}{}</div>",
            if bCanUndelete {
                format!(
                    " [<a href=\"/undelete?msgid={}\">\u{0432}\u{043e}\u{0441}\u{0441}\u{0442}\u{0430}\u{043d}\u{043e}\u{0432}\u{0438}\u{0442}\u{044c}</a>]",
                    stTopic.id
                )
            } else {
                String::new()
            }
        )
    } else {
        String::new()
    };

    let sEditSummaryHtml = if stInput.include_canonical_extras {
        sTopicEditSummaryHtml(stState, stTopic, optUser, stMeta.bExpired).await?
    } else {
        String::new()
    };
    let sWarningsHtml = if stInput.include_canonical_extras && !stMeta.bExpired {
        match optUser {
            Some(stUser) if stUser.canmod || stUser.corrector => {
                sTopicWarningsHtml(
                    stState,
                    stTopic.id,
                    stUser,
                    sCsrfToken,
                    stMeta.iOpenWarnings,
                )
                .await?
            }
            _ => String::new(),
        }
    } else {
        String::new()
    };
    let (sMemoriesButtonsHtml, sMemoriesScriptHtml) = if stInput.include_canonical_extras {
        stTopicMemoriesHtml(stState, stTopic.id, optUser, sCsrfToken).await?
    } else {
        (String::new(), String::new())
    };
    let iEffectivePostScore = iEffectiveTopicPostScore(stTopic, &stMeta);
    let bCanResolve = optUser.as_ref().is_some_and(|stUser| {
        stMeta.bResolvable && (stUser.canmod || stUser.id == stTopic.author_id)
    });
    let bCanWarn = optUser.as_ref().is_some_and(|stUser| {
        stUser.score.unwrap_or(0) >= 50
            && !stInput.actor_frozen
            && !stTopic.deleted
            && !stMeta.bExpired
            && !stTopic.draft
    });
    let bResolved = stTopic.resolved.unwrap_or(false);
    let bModeratorMenu = stInput.show_menu && bModerator && !stTopic.deleted;
    StTopicCardTemplate {
        card: StTopicCardView {
            topic: stInput.topic,
            title_plain: stInput.title_plain,
            topic_author_signature: stInput.topic_author_signature,
            topic_html: stInput.topic_html,
            poll: stInput.poll,
            images_html: stInput.images_html,
            topic_reactions_html: stInput.topic_reactions.html,
            topic_show_reactions_link: stInput.topic_reactions.show_menu_link,
            show_menu: stInput.show_menu,
            enable_schema: stInput.enable_schema,
            links_allowed: stMeta.bLinksAllowed,
            topic_expired: stMeta.bExpired,
            resolved: bResolved,
            moderator_menu: bModeratorMenu,
            can_commit: bCanCommit,
            can_comment: stInput.can_comment,
            can_edit: bCanEdit,
            can_delete: bCanDelete,
            can_resolve: bCanResolve,
            can_warn: bCanWarn,
            show_postscore: bAuthorized && !stMeta.bExpired,
            postscore_info_html: crate::domain::topic::options::sPostScoreInfo(iEffectivePostScore),
            deleted_header_html: sDeletedHeaderHtml,
            userpic_html: stInput.userpic_html,
            author_html: sAuthorHtml,
            remark_html: sRemarkHtml,
            moderator_ip_html: sModeratorIpHtml,
            committer_html: sCommitterHtml,
            edit_summary_html: sEditSummaryHtml,
            moderator_user_agent_html: sModeratorUserAgentHtml,
            warnings_html: sWarningsHtml,
            memories_buttons_html: sMemoriesButtonsHtml,
            memories_script_html: sMemoriesScriptHtml,
        },
    }
    .render()
    .map_err(AppError::Template)
}

/// Full `PreparedTopic` card used by `/uncommit.jsp`.  Java passes
/// `showMenu=false`, no `messageMenu`, no edit summary and no memories model,
/// but still prepares the author remark, committer, moderator IP/UA,
/// postscore, reactions, images and poll.
pub(crate) async fn sPrepareTopicCardHtml(
    stState: &AppState,
    iTopicId: i32,
    optUser: &Option<UserSummary>,
    sCsrfToken: &str,
    bShowMenu: bool,
) -> Result<String> {
    let stTopic = get_topic(stState, iTopicId).await?;
    let stMarkupUsers = stState
        .markup
        .stResolveBatch(std::iter::once((
            stTopic.message.as_str(),
            stTopic.markup.as_str(),
        )))
        .await?;
    let sTopicHtml = markup::render_topic_with_expanded_cut_policy_and_users(
        &stTopic.message,
        &stTopic.markup,
        stTopic.bNofollowAuthorLinks(),
        Some(&stState.config.public_url),
        Some(&stMarkupUsers),
    );
    let stMeta = stTopicCardMeta(
        stState,
        stTopic.id,
        optUser.as_ref().map(|stUser| stUser.id),
    )
    .await?;
    let (iScore, iMaxScore, bRegistered): (i32, i32, bool) = sqlx::query_as(
        "SELECT COALESCE(score,0),COALESCE(max_score,0),COALESCE(passwd,'')<>'' FROM users WHERE id=$1",
    )
    .bind(stTopic.author_id)
    .fetch_one(&stState.pool)
    .await?;
    let bModeratorSession = optUser.as_ref().is_some_and(|stUser| stUser.canmod);
    let bReactorFrozen = match optUser {
        Some(stUser) => sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT frozen_until FROM users WHERE id=$1",
        )
        .bind(stUser.id)
        .fetch_one(&stState.pool)
        .await?
        .is_some_and(|dtUntil| dtUntil > chrono::Utc::now()),
        None => false,
    };
    let vecAllReactions = load_all_reactions(
        stState,
        stTopic.id,
        optUser.as_ref().map(|stUser| stUser.id),
    )
    .await?;
    let vecTopicReactions = vecAllReactions
        .iter()
        .filter(|(optCommentId, ..)| optCommentId.is_none())
        .map(|(_, sReaction, iUserId, sNick, iScore)| {
            (sReaction.clone(), *iUserId, sNick.clone(), *iScore)
        })
        .collect::<Vec<_>>();
    let bAllowReactions = reactions_allow_interact(
        optUser,
        bReactorFrozen,
        stMeta.bExpired,
        stTopic.author_id,
        stTopic.deleted,
        false,
    );
    let stReactions = render_reactions_widget(
        stTopic.id,
        None,
        &vecTopicReactions,
        optUser.as_ref().map(|stUser| stUser.id),
        bAllowReactions,
        sCsrfToken,
    );
    let optPoll = load_poll_view(
        stState,
        stTopic.id,
        stTopic.deleted,
        poll_is_pending(stTopic.moderate),
        stMeta.bExpired,
        false,
        optUser,
        sCsrfToken,
        &stTopic.topic_url(),
    )
    .await?;
    let vecImages = load_topic_images(stState, stTopic.id).await?;
    let sImagesHtml = render_topic_images(
        &vecImages,
        &stTopic.title,
        stTopic.section_prefix == "gallery",
        false,
    );
    let sTitlePlain = stTopic.sTitlePlain();
    sBuildTopicCardHtml(
        stState,
        optUser,
        sCsrfToken,
        StTopicCardBuildInput {
            topic: stTopic,
            title_plain: sTitlePlain,
            topic_author_signature: stAuthorSignature(
                iScore,
                iMaxScore,
                bRegistered,
                bModeratorSession,
            ),
            topic_html: sTopicHtml,
            poll: optPoll,
            images_html: sImagesHtml,
            topic_reactions: stReactions,
            userpic_html: String::new(),
            can_comment: false,
            actor_frozen: bReactorFrozen,
            show_menu: bShowMenu,
            enable_schema: false,
            include_canonical_extras: false,
            remote_ip: String::new(),
        },
    )
    .await
}

fn sRealtimeTopicBootstrap(
    bEnabled: bool,
    iTopicId: i32,
    sTopicLink: &str,
    iLastCommentId: i32,
    sWsUrl: &str,
) -> String {
    if !bEnabled {
        return String::new();
    }
    let sTopicLink = serde_json::to_string(sTopicLink).expect("serializing a string cannot fail");
    let sWsUrl = serde_json::to_string(sWsUrl).expect("serializing a string cannot fail");
    format!(
        r#"<script>$script.ready('realtime', function() {{ RealtimeContext.setupTopic({iTopicId}, {sTopicLink}, {iLastCommentId}); RealtimeContext.start({sWsUrl}); }});</script>"#
    )
}

#[cfg(test)]
mod realtime_browser_contract_tests {
    use super::sRealtimeTopicBootstrap;
    use sha2::{Digest, Sha256};

    #[test]
    fn topic_bootstrap_matches_the_original_client_contract() {
        assert!(sRealtimeTopicBootstrap(false, 42, "/forum/lor/42", 7, "wss://lor/").is_empty());

        let sHtml = sRealtimeTopicBootstrap(
            true,
            42,
            "/forum/linux-org-ru/42",
            9001,
            "wss://www.linux.org.ru/",
        );
        assert!(sHtml.contains("$script.ready('realtime'"));
        assert!(sHtml.contains("RealtimeContext.setupTopic(42, \"/forum/linux-org-ru/42\", 9001)"));
        assert!(sHtml.contains("RealtimeContext.start(\"wss://www.linux.org.ru/\")"));
    }

    #[test]
    fn page_dom_loads_the_original_client_in_dependency_order() {
        let sBase = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/base.html"));
        let sTopic = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/topic.html"));
        let iScriptLoader = sBase.find("/js/script.min.js").expect("script.js loader");
        let iJquery = sBase
            .find("/webjars/jquery/3.7.1/jquery.min.js")
            .expect("original jQuery WebJar URL");
        let iLor = sBase
            .find("$script('/js/lor.js'")
            .expect("original LOR bundle");
        let iPlugins = sBase
            .find("$script('/js/plugins.js'")
            .expect("original plugin bundle");
        let iRealtime = sBase.find("/js/realtime.js").expect("realtime client");
        assert!(iScriptLoader < iJquery && iJquery < iLor && iLor < iRealtime);
        assert!(iJquery < iPlugins);
        assert!(sBase.contains("$script.ready('lorjs'"));
        assert!(sBase.contains("fixTimezone('<!-- LOR_TIMEZONE -->')"));
        assert!(sTopic.contains("data-format=\"default\""));
        assert_eq!(sTopic.matches("id=\"realtime\"").count(), 1);
        assert!(sTopic.contains("{{ realtime_bootstrap_html|safe }}"));
        assert!(sTopic.contains("<div class=\"userpic\"><img class=\"photo\""));
        assert!(sTopic.contains("message-w-userpic"));
        assert!(sTopic.contains("Ответ на:"));
        assert!(sTopic.contains("c.answer_count == 1"));
        assert!(!sTopic.contains("<h2>Комментарии:"));

        let sArchive = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/archive_index.html"
        ));
        assert!(sArchive.contains("action=\"/search.jsp\""));
        assert!(sArchive.contains("name=\"section\""));
        assert!(sArchive.contains("href=\"{{ archive_url }}\""));

        let sNewsCard = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/news_card.html"
        ));
        assert!(sNewsCard.contains("{% if t.can_comment %}"));
        assert!(sNewsCard.contains("comment-message.jsp?topic={{ t.topic.id }}"));
    }

    #[test]
    fn browser_loader_and_realtime_assets_are_byte_exact_java_copies() {
        let arrScript = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/script.min.js"
        ));
        let arrRealtime = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/realtime.js"
        ));
        let arrAddForm = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/add-form.js"
        ));
        assert_eq!(
            Sha256::digest(arrScript)
                .iter()
                .map(|iByte| format!("{iByte:02x}"))
                .collect::<String>(),
            "09fae4a64dbdfee232042ae76eb3e03f1521b9ecef352c6e8a1b6656c2a55c64"
        );
        assert_eq!(
            Sha256::digest(arrRealtime)
                .iter()
                .map(|iByte| format!("{iByte:02x}"))
                .collect::<String>(),
            "1665374fa67a2fc27681c6bb9ac92017ef2dbc78539cf947bfb050c70ddfb10a"
        );
        assert_eq!(
            Sha256::digest(arrAddForm)
                .iter()
                .map(|iByte| format!("{iByte:02x}"))
                .collect::<String>(),
            "daddf3c57a828e77fa96bdc17c01178e7aea560dd0a5a0b2d1e0121c419983b5"
        );
    }
}

#[derive(Debug, Clone)]
struct CommentView {
    item: CommentItem,
    author_signature: AuthorSignatureView,
    userpic_url: Option<String>,
    userpic_width: i32,
    userpic_height: i32,
    reply: Option<CommentReplyView>,
    answer_count: usize,
    answer_url: String,
    html: String,
    reactions_html: String,
    show_reactions_link: bool,
    can_edit: bool,
    can_delete: bool,
    can_undelete: bool,
    can_warn: bool,
    is_topic_author: bool,
    delete_info: Option<CommentDeleteInfoView>,
    author_readonly: bool,
}

#[derive(Debug, Clone)]
struct CommentDeleteInfoView {
    user_id: i32,
    nick: String,
    reason: String,
}

#[derive(Debug, Clone)]
struct CommentReplyView {
    id: i32,
    title: Option<String>,
    author: String,
    postdate: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default)]
struct AuthorSignatureView {
    stars_html: String,
    score: i32,
    max_score: i32,
    show_score: bool,
}

type TyAuthorPresentationRow = (i32, i32, i32, bool, Option<String>, Option<String>);

/// `User.getStars`: at most five filled stars for current score plus hollow
/// stars up to the historical maximum. Both values are capped at 599 before
/// converting each full hundred points into a star.
fn sUserStarsHtml(iScore: i32, iMaxScore: i32) -> String {
    let iNormalizedScore = iScore.clamp(0, 599);
    let iNormalizedMaxScore = iMaxScore.max(iScore).clamp(0, 599);
    let iGreenStars = iNormalizedScore / 100;
    let iGreyStars = iNormalizedMaxScore / 100 - iGreenStars;
    format!(
        "<span class=\"stars\">{}{}</span>",
        "★".repeat(iGreenStars as usize),
        "☆".repeat(iGreyStars as usize)
    )
}

fn stAuthorSignature(
    iScore: i32,
    iMaxScore: i32,
    bRegistered: bool,
    bModeratorSession: bool,
) -> AuthorSignatureView {
    AuthorSignatureView {
        stars_html: if bRegistered {
            sUserStarsHtml(iScore, iMaxScore)
        } else {
            String::new()
        },
        score: iScore,
        max_score: iMaxScore,
        show_score: bRegistered && bModeratorSession,
    }
}

/// poll-form.tag rendered server-side: a topic's poll (if any), with vote
/// counts/percentages and whether the current viewer may still vote.
/// `can_vote` doesn't pre-check expiry (Topic.expired isn't loaded by
/// `get_topic` here) - `/vote.jsp` itself still rejects an expired poll,
/// so a stale "Голосовать" button just surfaces that error instead of
/// silently vanishing a beat early.
#[derive(Debug, Clone)]
pub(crate) struct PollView {
    pub(crate) voteid: i32,
    pub(crate) multiselect: bool,
    pub(crate) variants: Vec<PollVariantView>,
    pub(crate) total_votes: i32,
    pub(crate) total_people: i64,
    pub(crate) can_vote: bool,
    pub(crate) show_results: bool,
    pub(crate) pending: bool,
    pub(crate) authorized: bool,
    pub(crate) topic_url: String,
    pub(crate) csrf_token: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PollVariantView {
    pub(crate) id: i32,
    pub(crate) label: String,
    pub(crate) votes: i32,
    pub(crate) pct: i32,
    pub(crate) progress_pct: i32,
    pub(crate) progress_alt: String,
    pub(crate) user_voted: bool,
}

/// PreparedImage-compatible view of any image attached to a topic.
#[derive(Debug, Clone)]
pub(crate) struct TopicImageView {
    pub(crate) id: i32,
    pub(crate) medium_url: String,
    pub(crate) original_url: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
    medium_width: i32,
    medium_height: i32,
    srcset: Vec<(String, i32)>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewsTopicView {
    pub(crate) topic: TopicSummary,
    pub(crate) topic_html: String,
    pub(crate) images_html: String,
    pub(crate) group_image_url: Option<String>,
    pub(crate) linktext: String,
    pub(crate) short_host: String,
    pub(crate) tags: Vec<NewsTagView>,
    pub(crate) show_group: bool,
    pub(crate) poll: Option<PollView>,
    pub(crate) minor: bool,
    pub(crate) pending: bool,
    /// `user-topics.jsp` uses `news.tag`, whose menu exposes edit/delete
    /// controls for drafts even outside the moderation queue.
    pub(crate) draft: bool,
    pub(crate) markup: String,
    pub(crate) postscore: i32,
    pub(crate) section_premoderated: bool,
    pub(crate) committed: bool,
    pub(crate) show_comments: bool,
    pub(crate) author_blocked: bool,
    pub(crate) author_link: bool,
    pub(crate) author_profile_url: String,
    pub(crate) author_remark: Option<String>,
    pub(crate) sign_date: chrono::DateTime<chrono::Utc>,
    /// `news.tag` is also used by the Java premoderation queue with
    /// `moderateMode=true`.  Keep that mode on the same prepared card so the
    /// queue cannot silently drift back to a compact topic list.
    pub(crate) moderate_mode: bool,
    pub(crate) can_commit: bool,
    pub(crate) can_delete: bool,
    pub(crate) can_edit: bool,
    /// Java `news.tag` renders the zero-comment action only when
    /// `messageMenu.commentsAllowed` is true for the current session.
    pub(crate) can_comment: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct NewsTagView {
    pub(crate) value: String,
    pub(crate) url: String,
}

fn bNewsAuthorLink(bAuthorAnonymous: bool, bSessionAuthorized: bool) -> bool {
    !bAuthorAnonymous || bSessionAuthorized
}

fn dtNewsSignDate(
    bSectionPremoderated: bool,
    bCommitted: bool,
    dtPostDate: chrono::DateTime<chrono::Utc>,
    optCommitDate: Option<chrono::DateTime<chrono::Utc>>,
) -> chrono::DateTime<chrono::Utc> {
    if bSectionPremoderated && bCommitted {
        optCommitDate.unwrap_or(dtPostDate)
    } else {
        dtPostDate
    }
}

#[cfg(test)]
mod news_signature_tests {
    use super::{bNewsAuthorLink, dtNewsSignDate};
    use chrono::{TimeZone, Utc};

    #[test]
    fn anonymous_author_is_plain_text_only_for_anonymous_viewer() {
        assert!(!bNewsAuthorLink(true, false));
        assert!(bNewsAuthorLink(true, true));
        assert!(bNewsAuthorLink(false, false));
    }

    #[test]
    fn premoderated_committed_topic_uses_commit_date() {
        let dtPostDate = Utc.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).unwrap();
        let dtCommitDate = Utc.with_ymd_and_hms(2026, 8, 2, 11, 0, 0).unwrap();

        assert_eq!(
            dtNewsSignDate(true, true, dtPostDate, Some(dtCommitDate)),
            dtCommitDate
        );
        assert_eq!(
            dtNewsSignDate(false, true, dtPostDate, Some(dtCommitDate)),
            dtPostDate
        );
        assert_eq!(
            dtNewsSignDate(true, false, dtPostDate, Some(dtCommitDate)),
            dtPostDate
        );
    }

    #[test]
    fn missing_commit_date_falls_back_to_post_date() {
        let dtPostDate = Utc.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).unwrap();
        assert_eq!(dtNewsSignDate(true, true, dtPostDate, None), dtPostDate);
    }

    #[test]
    fn news_signature_dom_preserves_blocked_link_and_private_remark_hooks() {
        let sTemplate = include_str!("../../templates/news_card.html");
        assert!(sTemplate.contains("{% if t.author_blocked %}<s>{% endif %}"));
        assert!(sTemplate.contains("{% if t.author_link %}<a itemprop=\"creator\""));
        assert!(sTemplate.contains("itemprop=\"datePublished\""));
        assert!(sTemplate.contains("itemprop=\"dateCreated\""));
        assert!(sTemplate.contains("<span class=\"user-remark\">{{ remark }}</span>"));
    }
}

fn sExternalLinkHost(sUrl: &str) -> String {
    let Some(sHost) = reqwest::Url::parse(sUrl)
        .ok()
        .and_then(|stUrl| stUrl.host_str().map(str::to_owned))
    else {
        return "Invalid URL, no host part!".to_owned();
    };
    psl::domain(sHost.as_bytes())
        .and_then(|stDomain| std::str::from_utf8(stDomain.as_bytes()).ok())
        .map(str::to_owned)
        .unwrap_or_else(|| "Invalid URL, no host part!".to_owned())
}

pub(crate) async fn load_topic_images(
    state: &AppState,
    topic_id: i32,
) -> Result<Vec<TopicImageView>> {
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT id, extension FROM images WHERE topic=$1 AND NOT deleted ORDER BY main DESC, id",
    )
    .bind(topic_id)
    .fetch_all(&state.pool)
    .await?;
    let mut prepared = Vec::with_capacity(rows.len());
    for (id, extension) in rows {
        let original = format!("images/{id}/original.{extension}");
        let path = format!(
            "{}/{}",
            state.config.upload_dir,
            original.trim_start_matches('/')
        );
        let pathMedium = format!("{}/images/{id}/1000px.jpg", state.config.upload_dir);
        let Some((width, height, medium_width, medium_height)) =
            tokio::task::spawn_blocking(move || {
                let (iWidth, iHeight) = image::image_dimensions(path).ok()?;
                let (iMediumWidth, iMediumHeight) = image::image_dimensions(pathMedium).ok()?;
                Some((
                    iWidth as i32,
                    iHeight as i32,
                    iMediumWidth as i32,
                    iMediumHeight as i32,
                ))
            })
            .await
            .unwrap_or(None)
        else {
            // ImageService.prepareImage logs and omits missing or corrupt
            // files instead of fabricating dimensions for them.
            continue;
        };
        let medium = format!("images/{id}/1000px.jpg");
        let srcset = [500, 1000, 1500, 2000]
            .into_iter()
            .map(|size| (format!("/images/{id}/{size}px.jpg"), size))
            .collect::<Vec<_>>();
        prepared.push(TopicImageView {
            id,
            medium_url: format!("/{medium}"),
            original_url: format!("/{original}"),
            width,
            height,
            medium_width,
            medium_height,
            srcset,
        });
    }
    Ok(prepared)
}

fn image_srcset(image: &TopicImageView) -> String {
    image
        .srcset
        .iter()
        .map(|(url, width)| format!("{url} {width}w"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn topic_image_srcset(image: &TopicImageView) -> String {
    image_srcset(image)
}

fn render_single_image(image: &TopicImageView, title: &str, imagepost: bool, news: bool) -> String {
    let height_limit = if news { "70vh" } else { "90vh" };
    let sizes = if news {
        "(min-width: 47em) 40vw, 100vw"
    } else {
        "(min-width: 70em) 80vw, 100vw"
    };
    let max_width = image.width.min(2000);
    let padding = 100.0 * f64::from(image.medium_height) / f64::from(image.medium_width);
    let title = html_escape::encode_double_quoted_attribute(title);
    let src = html_escape::encode_double_quoted_attribute(&image.medium_url);
    let original = html_escape::encode_double_quoted_attribute(&image.original_url);
    let srcset_value = image_srcset(image);
    let srcset = html_escape::encode_double_quoted_attribute(&srcset_value);
    let linked = imagepost || image.width >= 1920 || image.height >= 1080;
    let open_link = if linked {
        format!(r#"<a href="{original}" itemprop="contentURL">"#)
    } else {
        String::new()
    };
    let close_link = if linked { "</a>" } else { "" };
    format!(
        r#"<div class="medium-image-container" style="max-width: {max_width}px; max-height: {height_limit}; width: min(var(--image-width), calc({height_limit} * {mw} / {mh}))">
<figure class="medium-image" style="position: relative; padding-bottom: {padding}%; padding-bottom: min({padding}%, {height_limit}); margin: 0" itemprop="associatedMedia" itemscope itemtype="http://schema.org/ImageObject">
{open_link}<img itemprop="thumbnail" class="medium-image" src="{src}" alt="{title}" srcset="{srcset}" sizes="{sizes}" style="position: absolute; max-height: {height_limit}" width="{mw}" height="{mh}">{close_link}
<meta itemprop="caption" content="{title}">
</figure></div>"#,
        mw = image.medium_width,
        mh = image.medium_height
    )
}

fn render_image_slider(images: &[TopicImageView], title: &str, news: bool) -> String {
    let main = &images[0];
    let height_limit = if news { "70vh" } else { "90vh" };
    let sizes = if news {
        "(min-width: 47em) 40vw, 100vw"
    } else {
        "(min-width: 70em) 80vw, 100vw"
    };
    let classes = if news {
        "slider-nav-autohide slider-nav-round slider-indicators-sm slider-indicators-outside"
    } else {
        "slider-indicators-outside slider-indicators-sm"
    };
    let title = html_escape::encode_double_quoted_attribute(title);
    let mut items = String::new();
    let mut indicators = String::new();
    for (index, image) in images.iter().enumerate() {
        let original = html_escape::encode_double_quoted_attribute(&image.original_url);
        let src = html_escape::encode_double_quoted_attribute(&image.medium_url);
        let srcset_value = image_srcset(image);
        let srcset = html_escape::encode_double_quoted_attribute(&srcset_value);
        let loading = if index == 0 { "" } else { " loading=\"lazy\"" };
        items.push_str(&format!(r#"<a href="{original}"><img src="{src}" alt="{title}" srcset="{srcset}" sizes="{sizes}" style="max-width: 100%; height: auto; max-height: 100%; top: 50%; transform: translateY(-50%)" width="{}" height="{}"{loading}></a>"#, image.medium_width, image.medium_height));
        indicators.push_str(&format!(
            r#"<a href="{original}"{}></a>"#,
            if index == 0 { " class=\"active\"" } else { "" }
        ));
    }
    format!(
        r#"<div class="slider-parent" style="width: min(var(--image-width), calc({height_limit} * {mw} / {mh}))">
<div class="swiffy-slider slider-indicators-round {classes} slider-item-ratio slider-item-ratio-contain" style="--swiffy-slider-item-ratio: {fw}/{fh}">
<div class="slider-container">{items}</div>
<button type="button" class="slider-nav" aria-label="Предыдущее изображение"></button>
<button type="button" class="slider-nav slider-nav-next" aria-label="Следующее изображение"></button>
<div class="slider-indicators">{indicators}</div>
</div></div>"#,
        mw = main.medium_width,
        mh = main.medium_height,
        fw = main.width,
        fh = main.height
    )
}

fn render_topic_images(
    images: &[TopicImageView],
    sStoredTitle: &str,
    imagepost: bool,
    news: bool,
) -> String {
    let sTitlePlain = crate::domain::title::sTopicTitlePlainForDisplay(sStoredTitle);
    render_topic_images_with_plain_title(images, &sTitlePlain, imagepost, news)
}

fn render_topic_images_with_plain_title(
    images: &[TopicImageView],
    sTitlePlain: &str,
    imagepost: bool,
    news: bool,
) -> String {
    match images {
        [] => String::new(),
        [image] => render_single_image(image, sTitlePlain, imagepost, news),
        _ => render_image_slider(images, sTitlePlain, news),
    }
}

#[cfg(test)]
mod image_view_tests {
    use super::*;

    fn image(id: i32) -> TopicImageView {
        TopicImageView {
            id,
            medium_url: format!("/images/{id}/1000px.jpg"),
            original_url: format!("/images/{id}/original.png"),
            width: 1920,
            height: 1080,
            medium_width: 800,
            medium_height: 450,
            srcset: vec![
                (format!("/images/{id}/500px.jpg"), 500),
                (format!("/images/{id}/1000px.jpg"), 1000),
                (format!("/images/{id}/1500px.jpg"), 1500),
                (format!("/images/{id}/2000px.jpg"), 2000),
            ],
        }
    }

    #[test]
    fn one_image_uses_the_original_responsive_container() {
        let html = render_topic_images(&[image(1)], "Заголовок", false, true);
        assert!(html.contains("medium-image-container"));
        assert!(html.contains("(min-width: 47em) 40vw, 100vw"));
        assert!(html.contains("500px.jpg 500w"));
        assert!(html.contains("2000px.jpg 2000w"));
        assert!(html.contains("max-height: 70vh"));
    }

    #[test]
    fn several_images_use_the_original_slider_dom() {
        let html = render_topic_images(&[image(1), image(2)], "Заголовок", false, false);
        assert!(html.contains("swiffy-slider"));
        assert!(html.contains("slider-nav-next"));
        assert!(html.contains("slider-indicators"));
        assert!(html.contains("/images/1/1000px.jpg"));
        assert!(html.contains("/images/2/1000px.jpg"));
    }

    #[test]
    fn stored_title_is_decoded_then_attribute_escaped_once() {
        let html = render_topic_images(
            &[image(1)],
            "A &amp; B &lt;b&gt; &quot;Q&quot; &#39;X&#39;",
            false,
            false,
        );

        assert!(html.contains("alt=\"A &amp; B &lt;b&gt; «Q» 'X'\""));
        assert!(html.contains("content=\"A &amp; B &lt;b&gt; «Q» 'X'\""));
        assert!(!html.contains("&amp;amp;"));
        assert!(!html.contains("<b>"));
    }
}

pub(crate) async fn prepare_news_topics(
    state: &AppState,
    topics: Vec<TopicSummary>,
    show_group: bool,
) -> Result<Vec<NewsTopicView>> {
    prepare_news_topics_for_viewer(state, topics, show_group, &None, "").await
}

pub(crate) async fn prepare_news_topics_for_viewer(
    state: &AppState,
    topics: Vec<TopicSummary>,
    show_group: bool,
    current_user: &Option<UserSummary>,
    csrf_token: &str,
) -> Result<Vec<NewsTopicView>> {
    let mut prepared = Vec::with_capacity(topics.len());
    let vecTopicIds = topics.iter().map(|stTopic| stTopic.id).collect::<Vec<_>>();
    let stMarkupUsers = state.markup.stResolveMessageIds(&vecTopicIds).await?;
    let mapAuthorRemarks: std::collections::HashMap<i32, String> = if let Some(stViewer) =
        current_user.as_ref()
    {
        let vecAuthorIds: Vec<i32> = topics
            .iter()
            .map(|stTopic| stTopic.author_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        sqlx::query_as::<_, (i32, String)>(
                "SELECT ref_user_id,remark_text FROM user_remarks WHERE user_id=$1 AND ref_user_id=ANY($2)",
            )
            .bind(stViewer.id)
            .bind(&vecAuthorIds)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    let stPostingResolution = crate::application::auth::stResolvePostingIdentity(
        state,
        current_user.as_ref(),
        None,
        None,
    )
    .await?;
    let stPostingIdentity = stPostingResolution.stIdentity;
    for topic in topics {
        type TyNewsTopicRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            bool,
            Option<chrono::DateTime<chrono::Utc>>,
            bool,
            bool,
            bool,
            i32,
            bool,
            bool,
            bool,
            bool,
            i32,
        );
        let row: Option<TyNewsTopicRow> = sqlx::query_as(
            r#"SELECT m.message, m.markup::text, t.linktext, g.image, t.moderate, t.commitdate,
                      COALESCE(t.commitdate,t.postdate)+s.expire<CURRENT_TIMESTAMP,
                      t.minor, s.moderate, COALESCE(u.score,0),
                      COALESCE(u.blocked,false), COALESCE(u.passwd,'')='',
                      COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false), t.draft,
                      COALESCE(t.postscore,-9999)
                 FROM msgbase m
                 JOIN topics t ON t.id=m.id
                 JOIN users u ON u.id=t.userid
                 JOIN groups g ON g.id=t.groupid
                 JOIN sections s ON s.id=g.section
                WHERE m.id=$1"#,
        )
        .bind(topic.id)
        .fetch_optional(&state.pool)
        .await?;
        let (
            message,
            message_markup,
            linktext,
            group_image,
            moderate,
            optCommitDate,
            expired,
            minor,
            section_premoderated,
            iAuthorScore,
            bAuthorBlocked,
            bAuthorAnonymous,
            bAuthorFrozen,
            bDraft,
            iPostscore,
        ) = row.unwrap_or_else(|| {
            (
                String::new(),
                "BBCODE_TEX".into(),
                None,
                None,
                false,
                None,
                false,
                false,
                false,
                0,
                false,
                true,
                false,
                false,
                -9999,
            )
        });
        let bNofollow = !crate::domain::topic::link_policy::StAuthorLinkState {
            iScore: iAuthorScore,
            bBlocked: bAuthorBlocked,
            bAnonymous: bAuthorAnonymous,
            bFrozen: bAuthorFrozen,
        }
        .bFollowInTopic(moderate);
        let dtSignDate = dtNewsSignDate(
            section_premoderated,
            moderate,
            topic.postdate,
            optCommitDate,
        );
        let images = load_topic_images(state, topic.id).await?;
        let images_html = render_topic_images(
            &images,
            &topic.title,
            topic.section_prefix == "gallery",
            true,
        );
        let group_image_url = group_image.map(|path| {
            if path.starts_with('/') {
                format!("/tango{path}")
            } else {
                format!("/tango/{path}")
            }
        });
        let poll = if topic.section_prefix == "polls" {
            load_poll_view(
                state,
                topic.id,
                topic.deleted,
                poll_is_pending(moderate),
                expired,
                false,
                current_user,
                csrf_token,
                &topic.topic_url(),
            )
            .await?
        } else {
            None
        };
        let short_host = topic
            .url
            .as_deref()
            .map(sExternalLinkHost)
            .unwrap_or_default();
        let tags = topic
            .tags_vec()
            .into_iter()
            .map(|value| NewsTagView {
                url: format!("/tag/{}", urlencoding::encode(&value)),
                value,
            })
            .collect();
        // The public topic/feed UI on LOR does not expose reply controls to an
        // anonymous viewer. Keep the legacy comment endpoints capable of
        // validating an explicitly supplied posting identity, but only offer
        // the browser action when the current session is authenticated.
        let can_comment = stPostingIdentity.bAuthorized
            && crate::routes::comments::check_comment_posting_allowed(
                state,
                &stPostingIdentity.stUser,
                false,
                topic.id,
            )
            .await
            .is_ok();
        prepared.push(NewsTopicView {
            topic_html: markup::render_topic_with_minimized_cut_policy_and_users(
                &message,
                &message_markup,
                &topic.topic_url(),
                bNofollow,
                Some(&state.config.public_url),
                Some(&stMarkupUsers),
            ),
            images_html,
            group_image_url,
            linktext: linktext
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Подробности".to_string()),
            short_host,
            tags,
            show_group,
            poll,
            minor,
            pending: section_premoderated && !moderate,
            draft: bDraft,
            markup: message_markup,
            postscore: iPostscore,
            section_premoderated,
            committed: moderate,
            show_comments: iPostscore != crate::domain::topic::posting::POSTSCORE_HIDE_COMMENTS,
            author_blocked: bAuthorBlocked,
            author_link: bNewsAuthorLink(bAuthorAnonymous, current_user.is_some()),
            author_profile_url: format!("/people/{}/profile", urlencoding::encode(&topic.author)),
            author_remark: mapAuthorRemarks.get(&topic.author_id).cloned(),
            sign_date: dtSignDate,
            moderate_mode: false,
            can_commit: false,
            can_delete: false,
            can_edit: false,
            can_comment,
            topic,
        });
    }
    Ok(prepared)
}

/// ReactionService.allowInteract: logged in, not frozen, not the target's
/// own author, target (topic/comment) not deleted, topic not expired, and
/// (for a comment) the topic's comments aren't hidden.
#[allow(clippy::too_many_arguments)]
fn reactions_allow_interact(
    current_user: &Option<UserSummary>,
    frozen: bool,
    topic_expired: bool,
    target_author_id: i32,
    target_deleted: bool,
    comments_hidden: bool,
) -> bool {
    match current_user {
        Some(u) => {
            u.id != target_author_id
                && !frozen
                && !topic_expired
                && !target_deleted
                && !comments_hidden
        }
        None => false,
    }
}

/// Server-rendered equivalent of `reactions.tag`.  A widget with no existing
/// reactions is hidden as a whole; the message menu receives a separate
/// "Реакции" link which reveals it.  Once at least one reaction exists, only
/// non-zero buttons are visible and the zero-count choices live behind `»`.
#[derive(Debug, Clone)]
struct ReactionsWidget {
    html: String,
    show_menu_link: bool,
}

fn render_reactions_widget(
    topic_id: i32,
    comment_id: Option<i32>,
    reaction_users: &[(String, i32, String, i32)],
    current_user_id: Option<i32>,
    allow_interact: bool,
    csrf_token: &str,
) -> ReactionsWidget {
    let is_anonymous = current_user_id.is_none();
    let anon_class = if is_anonymous {
        " reaction-anonymous"
    } else {
        ""
    };
    let disabled = if allow_interact { "" } else { " disabled" };

    struct Btn {
        emoji: String,
        count: i64,
        clicked: bool,
        tooltip: String,
    }
    let mut buttons = Vec::new();
    for (emoji, description) in crate::routes::api::REACTIONS {
        let mut users: Vec<&(String, i32, String, i32)> =
            reaction_users.iter().filter(|(r, ..)| r == emoji).collect();
        users.sort_by_key(|stUser| std::cmp::Reverse(stUser.3));
        let count = users.len() as i64;
        let clicked = current_user_id
            .map(|uid| users.iter().any(|(_, u, ..)| *u == uid))
            .unwrap_or(false);
        let top: Vec<&str> = users
            .iter()
            .take(3)
            .map(|(_, _, nick, _)| nick.as_str())
            .collect();
        let more = if users.len() > 3 { "..." } else { "" };
        let tooltip = format!("Реакция \"{description}\": {}{more}", top.join(" "));
        buttons.push(Btn {
            emoji: emoji.to_string(),
            count,
            clicked,
            tooltip,
        });
    }

    // PreparedReactions uses a TreeMap, so preserve the original UTF-16
    // string order rather than the declaration order of REACTIONS.
    buttons.sort_by_key(|button| button.emoji.encode_utf16().collect::<Vec<_>>());

    let has_reactions = buttons.iter().any(|button| button.count > 0);
    let outer_class = if has_reactions {
        "reactions"
    } else {
        "reactions zero-reactions"
    };

    let mut html = format!(
        "<div class=\"{outer_class}\"><form class=\"reactions-form\" action=\"/reactions\" method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"topic\" value=\"{topic_id}\">",
        html_escape::encode_double_quoted_attribute(csrf_token),
    );
    if let Some(cid) = comment_id {
        html.push_str(&format!(
            "<input type=\"hidden\" name=\"comment\" value=\"{cid}\">"
        ));
    }
    for b in buttons.iter().filter(|button| button.count > 0) {
        let value = format!("{}-{}", b.emoji, !b.clicked);
        let clicked_class = if b.clicked { " btn-primary" } else { "" };
        html.push_str(&format!(
            "<button name=\"reaction\" value=\"{}\" class=\"reaction{clicked_class}{anon_class}\" title=\"{}\"{disabled}>{} <span class=\"reaction-count\">{}</span></button>",
            html_escape::encode_double_quoted_attribute(&value),
            html_escape::encode_double_quoted_attribute(&b.tooltip),
            html_escape::encode_text(&b.emoji),
            b.count,
        ));
    }
    if has_reactions && current_user_id.is_some() {
        let comment_query = comment_id
            .map(|id| format!("&comment={id}"))
            .unwrap_or_default();
        html.push_str(&format!(
            "<a class=\"reaction reaction-show-list\" href=\"/reactions?topic={topic_id}{comment_query}\">?</a>",
        ));
    }
    if allow_interact && buttons.iter().any(|button| button.count == 0) {
        if has_reactions {
            let comment_query = comment_id
                .map(|id| format!("&comment={id}"))
                .unwrap_or_default();
            html.push_str(&format!(
                "<a class=\"reaction reaction-show\" href=\"/reactions?topic={topic_id}{comment_query}\">&raquo;</a><span class=\"zero-reactions\">",
            ));
        }
        for b in buttons.iter().filter(|button| button.count == 0) {
            html.push_str(&format!(
                "<button name=\"reaction\" value=\"{}-true\" class=\"reaction{anon_class}\" title=\"{}\">{} <span class=\"reaction-count\">0</span></button>",
                html_escape::encode_double_quoted_attribute(&b.emoji),
                html_escape::encode_double_quoted_attribute(&b.tooltip),
                html_escape::encode_text(&b.emoji),
            ));
        }
        if has_reactions {
            html.push_str("</span>");
        }
    }
    html.push_str("</form></div>");
    ReactionsWidget {
        html,
        show_menu_link: !has_reactions && allow_interact,
    }
}

#[cfg(test)]
mod reactions_widget_tests {
    use super::*;

    #[test]
    fn empty_reactions_are_hidden_and_revealed_from_the_message_menu() {
        let widget = render_reactions_widget(42, None, &[], Some(7), true, "token");

        assert!(widget.show_menu_link);
        assert!(
            widget
                .html
                .starts_with("<div class=\"reactions zero-reactions\">")
        );
        assert!(!widget.html.contains("class=\"reaction reaction-show\""));
        assert!(widget.html.contains("name=\"reaction\""));
        assert!(
            widget
                .html
                .contains("<span class=\"reaction-count\">0</span>")
        );
    }

    #[test]
    fn existing_reactions_show_counts_and_collapse_zero_choices() {
        let rows = vec![
            ("👍".to_string(), 10, "alice".to_string(), 100),
            ("🎉".to_string(), 11, "bob".to_string(), 50),
        ];
        let widget = render_reactions_widget(42, Some(9), &rows, Some(10), true, "token");

        assert!(!widget.show_menu_link);
        assert!(widget.html.starts_with("<div class=\"reactions\">"));
        assert!(
            widget
                .html
                .contains("href=\"/reactions?topic=42&comment=9\">?</a>")
        );
        assert!(widget.html.contains("class=\"reaction reaction-show\""));
        assert!(widget.html.contains("<span class=\"zero-reactions\">"));
        assert!(widget.html.find("🎉").unwrap() < widget.html.find("👍").unwrap());
    }

    #[test]
    fn anonymous_empty_widget_has_no_reveal_link_or_buttons() {
        let widget = render_reactions_widget(42, None, &[], None, false, "token");

        assert!(!widget.show_menu_link);
        assert!(
            widget
                .html
                .starts_with("<div class=\"reactions zero-reactions\">")
        );
        assert!(!widget.html.contains("name=\"reaction\""));
    }
}

/// All reactions for the topic in one query (topic-level rows have
/// `comment_id IS NULL`), so per-comment widgets don't each hit the DB.
/// The JSON maps on topics/comments are the authoritative state in Java;
/// reactions_log is only an audit/date source and can be incomplete in an
/// imported database.
async fn load_all_reactions(
    state: &AppState,
    topic_id: i32,
    optViewerId: Option<i32>,
) -> Result<Vec<(Option<i32>, String, i32, String, i32)>> {
    Ok(sqlx::query_as(
        r#"SELECT NULL::integer AS comment_id, item.value AS reaction,
                  item.key::integer AS origin_user, u.nick, COALESCE(u.score,0)
           FROM topics t
           CROSS JOIN LATERAL jsonb_each_text(COALESCE(t.reactions,'{}'::jsonb)) item
           JOIN users u ON u.id=item.key::integer
           WHERE t.id=$1 AND item.key ~ '^[0-9]+$'
             AND ($2::int IS NULL OR NOT EXISTS (
               SELECT 1 FROM ignore_list il
               WHERE il.userid=$2 AND il.ignored=item.key::integer
             ))
           UNION ALL
           SELECT c.id AS comment_id, item.value AS reaction,
                  item.key::integer AS origin_user, u.nick, COALESCE(u.score,0)
           FROM comments c
           CROSS JOIN LATERAL jsonb_each_text(COALESCE(c.reactions,'{}'::jsonb)) item
           JOIN users u ON u.id=item.key::integer
           WHERE c.topic=$1 AND item.key ~ '^[0-9]+$'
             AND ($2::int IS NULL OR NOT EXISTS (
               SELECT 1 FROM ignore_list il
               WHERE il.userid=$2 AND il.ignored=item.key::integer
             ))"#,
    )
    .bind(topic_id)
    .bind(optViewerId)
    .fetch_all(&state.pool)
    .await?)
}

async fn load_poll_view(
    state: &AppState,
    topic_id: i32,
    deleted: bool,
    pending: bool,
    expired: bool,
    results_requested: bool,
    current_user: &Option<UserSummary>,
    csrf_token: &str,
    topic_url: &str,
) -> Result<Option<PollView>> {
    let Some((poll_id, multiselect)): Option<(i32, bool)> =
        sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1")
            .bind(topic_id)
            .fetch_optional(&state.pool)
            .await?
    else {
        return Ok(None);
    };
    let current_user_id = current_user.as_ref().map(|user| user.id).unwrap_or(0);
    let mut rows: Vec<(i32, String, i32, bool)> = sqlx::query_as(
        "SELECT v.id,v.label,v.votes,EXISTS(SELECT 1 FROM vote_users u WHERE u.vote=v.vote AND u.variant_id=v.id AND u.userid=$2) FROM polls_variants v WHERE v.vote=$1 ORDER BY v.id",
    ).bind(poll_id).bind(current_user_id).fetch_all(&state.pool).await?;
    let total_votes: i32 = rows.iter().map(|(_, _, votes, _)| *votes).sum();
    let total_people: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT userid) FROM vote_users WHERE vote=$1")
            .bind(poll_id)
            .fetch_one(&state.pool)
            .await?;
    let user_voted = rows.iter().any(|row| row.3);
    let show_results = !pending && (results_requested || user_voted || expired);
    if show_results {
        rows.sort_by_key(|(id, _, votes, _)| (std::cmp::Reverse(*votes), *id));
    }
    let max_votes = rows.iter().map(|row| row.2).max().unwrap_or(0);
    let divisor = iPollPercentageDivisor(multiselect, total_votes, total_people, max_votes);
    let variants = rows
        .into_iter()
        .map(|(id, label, votes, selected)| {
            let iWidth = if max_votes > 0 {
                320 * votes / max_votes
            } else {
                0
            };
            PollVariantView {
                id,
                label,
                votes,
                pct: if divisor > 0 {
                    ((100.0 * f64::from(votes) / f64::from(divisor)).round()) as i32
                } else {
                    0
                },
                progress_pct: (iWidth / 16) * 16 * 100 / 320,
                progress_alt: "*".repeat(iWidth as usize),
                user_voted: selected,
            }
        })
        .collect();
    let authorized = current_user.is_some();
    Ok(Some(PollView {
        voteid: poll_id,
        multiselect,
        variants,
        total_votes,
        total_people,
        can_vote: authorized && !user_voted && !deleted && !pending && !expired,
        show_results,
        pending,
        authorized,
        topic_url: topic_url.to_owned(),
        csrf_token: csrf_token.to_owned(),
    }))
}

fn iPollPercentageDivisor(
    bMultiselect: bool,
    iTotalVotes: i32,
    iTotalPeople: i64,
    iMaxVotes: i32,
) -> i32 {
    if bMultiselect {
        i32::try_from(iTotalPeople)
            .unwrap_or(i32::MAX)
            .max(iMaxVotes)
    } else {
        iTotalVotes
    }
}

fn poll_is_pending(topic_committed: bool) -> bool {
    !topic_committed
}

#[cfg(test)]
mod external_link_host_tests {
    use super::sExternalLinkHost;

    #[test]
    fn uses_public_suffix_list_like_guava_top_private_domain() {
        assert_eq!(
            sExternalLinkHost("https://www.linux.org.ru/news/1"),
            "linux.org.ru"
        );
        assert_eq!(
            sExternalLinkHost("https://docs.example.co.uk/path"),
            "example.co.uk"
        );
        assert_eq!(sExternalLinkHost("not a URL"), "Invalid URL, no host part!");
    }
}

#[cfg(test)]
mod poll_moderation_semantics_tests {
    use super::{iPollPercentageDivisor, poll_is_pending};

    #[test]
    fn poll_is_pending_until_topics_moderate_is_true() {
        assert!(poll_is_pending(false));
        assert!(!poll_is_pending(true));
    }

    #[test]
    fn single_select_percentages_survive_incomplete_legacy_vote_users() {
        assert_eq!(iPollPercentageDivisor(false, 393, 1, 220), 393);
    }

    #[test]
    fn multiselect_percentages_use_people_but_never_exceed_one_hundred() {
        assert_eq!(iPollPercentageDivisor(true, 8, 6, 3), 6);
        assert_eq!(iPollPercentageDivisor(true, 393, 1, 220), 220);
    }
}

#[cfg(test)]
mod author_signature_tests {
    use super::{sUserStarsHtml, stAuthorSignature};

    #[test]
    fn stars_match_java_score_and_historical_maximum_rules() {
        assert_eq!(sUserStarsHtml(45, 45), "<span class=\"stars\"></span>");
        assert_eq!(sUserStarsHtml(201, 350), "<span class=\"stars\">★★☆</span>");
        assert_eq!(
            sUserStarsHtml(3_000, 3_200),
            "<span class=\"stars\">★★★★★</span>"
        );
    }

    #[test]
    fn anonymous_authors_have_no_star_wrapper_or_moderator_score() {
        let stSignature = stAuthorSignature(500, 500, false, true);
        assert!(stSignature.stars_html.is_empty());
        assert!(!stSignature.show_score);
    }
}

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
    form_error: Option<String>,
    topic_limit_error: Option<String>,
    topic_limit_info: Option<String>,
    topic_posting_allowed: bool,
    topic_posting_reason: String,
    action: String,
    topic_id: Option<i32>,
    csrf_token: String,
    /// Existing (id, label) poll variants - empty for a brand-new topic.
    poll_variants: Vec<(i32, String)>,
    /// Number of blank "add new variant" rows to render: `Poll.MaxPollSize`
    /// (15) for a new topic, `EditTopicRequest.newPoll`'s size (3) for edit.
    poll_new_rows: Vec<String>,
    poll_multiselect: bool,
    selected_group: i32,
    is_edit: bool,
    links_allowed: bool,
    premoderated: bool,
    poll_allowed: bool,
    image_allowed: bool,
    image_required: bool,
    additional_image_rows: Vec<()>,
    existing_images: Vec<TopicImageView>,
    uploaded_images: Vec<String>,
    form_title: String,
    form_msg: String,
    form_url: String,
    form_linktext: String,
    form_tags: String,
    preview_html: Option<String>,
    noinfo: bool,
    add_info_html: Option<String>,
    format_mode: String,
    format_mode_title: String,
    anonymous_form: bool,
    form_nick: String,
    require_captcha: bool,
    captcha_site_key: String,
    show_allow_anonymous: bool,
    allow_anonymous: bool,
}

async fn user_format_mode(state: &AppState, user_id: i32) -> Result<(String, String, String)> {
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let mode = crate::profile::ProfileSettings::from_hstore_text(settings_text).format_mode;
    let title = crate::profile::FORMAT_MODES
        .iter()
        .find(|(id, _, _)| *id == mode)
        .map(|(_, title, _)| *title)
        .unwrap_or("Markdown")
        .to_string();
    let markup = match mode.as_str() {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "plain" => "PLAIN",
        _ => "BBCODE_TEX",
    };
    Ok((mode, title, markup.to_string()))
}

pub(crate) fn markup_form_view(markup: &str) -> (String, String) {
    match markup {
        "MARKDOWN" => ("markdown".into(), "Markdown".into()),
        "BBCODE_ULB" => ("ntobr".into(), "User line break".into()),
        "PLAIN" => ("plain".into(), "HTML".into()),
        "BBCODE_TEX" | "LORCODE" => ("lorcode".into(), "LORCODE".into()),
        _ => ("lorcode".into(), "LORCODE".into()),
    }
}

#[derive(Debug, Clone)]
struct AddSectionChoice {
    title: String,
    url: String,
    view_url: Option<String>,
    info: Option<String>,
    postable: bool,
    reason: String,
}

#[derive(Template)]
#[template(path = "add_section.html")]
struct AddSectionTemplate {
    title: String,
    heading: String,
    choices: Vec<AddSectionChoice>,
    choosing_groups: bool,
}

pub struct TopicForm {
    pub group: i32,
    pub title: String,
    pub msg: String,
    pub url: Option<String>,
    pub linktext: Option<String>,
    pub tags: Option<String>,
    pub draft: Option<String>,
    pub preview: Option<String>,
    pub noinfo: Option<String>,
    pub poll: Vec<String>,
    pub multiselect: Option<String>,
    pub nick: Option<String>,
    pub password: Option<String>,
    pub captcha_response: Option<String>,
    pub allow_anonymous: Option<String>,
    pub uploaded_images: Vec<String>,
}

/// `axum::Form` can't deserialize repeated/indexed poll keys into `Vec`
/// fields (see `crate::form`), so this form is parsed from the raw body.
fn parse_indexed_field(pairs: &[(String, String)], prefix: &str) -> Vec<(i32, String)> {
    let start = format!("{prefix}[");
    let mut values: Vec<(i32, String)> = pairs
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(&start)?
                .strip_suffix(']')?
                .parse()
                .ok()
                .map(|index| (index, value.clone()))
        })
        .collect();
    values.sort_by_key(|(index, _)| *index);
    values
}

fn parse_topic_form(pairs: &[(String, String)]) -> Result<TopicForm> {
    use crate::form::{get, get_all};
    let indexed_poll = parse_indexed_field(pairs, "poll");
    let new_poll = parse_indexed_field(pairs, "newPoll");
    let poll = if !indexed_poll.is_empty() || !new_poll.is_empty() {
        let mut labels = indexed_poll
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>();
        labels.extend(new_poll.into_iter().map(|(_, label)| label));
        labels
    } else {
        // Accept the first Rust port's flattened fields as a compatibility
        // fallback, while every generated form uses Java's indexed names.
        get_all(pairs, "poll")
            .into_iter()
            .map(str::to_string)
            .collect()
    };
    Ok(TopicForm {
        group: get(pairs, "group")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        title: get(pairs, "title").unwrap_or("").to_string(),
        msg: get(pairs, "msg").unwrap_or("").to_string(),
        url: get(pairs, "url").map(|s| s.to_string()),
        linktext: get(pairs, "linktext").map(|s| s.to_string()),
        tags: get(pairs, "tags").map(|s| s.to_string()),
        draft: get(pairs, "draft").map(|s| s.to_string()),
        preview: get(pairs, "preview").map(str::to_string),
        noinfo: get(pairs, "noinfo").map(str::to_string),
        poll,
        multiselect: get(pairs, "multiselect")
            .or_else(|| get(pairs, "multiSelect"))
            .map(str::to_string),
        nick: get(pairs, "nick").map(str::to_string),
        password: get(pairs, "password").map(str::to_string),
        captcha_response: get(pairs, "h-captcha-response").map(str::to_string),
        allow_anonymous: get(pairs, "allowAnonymous").map(str::to_string),
        uploaded_images: parse_indexed_field(pairs, "uploadedImages")
            .into_iter()
            .map(|(_, sName)| sName)
            .filter(|sName| !sName.trim().is_empty())
            .collect(),
    })
}

#[cfg(test)]
mod topic_form_contract_tests {
    use super::*;

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn parses_java_add_topic_poll_contract() {
        let form = parse_topic_form(&pairs(&[
            ("group", "19387"),
            ("title", "Опрос"),
            ("msg", "Текст"),
            ("tags", "lor"),
            ("poll[1]", "Второй"),
            ("poll[0]", "Первый"),
            ("multiSelect", "true"),
        ]))
        .unwrap();
        assert_eq!(form.group, 19387);
        assert_eq!(form.poll, ["Первый", "Второй"]);
        assert!(form.multiselect.is_some());
    }

    #[test]
    fn accepts_legacy_flattened_rust_fields_during_transition() {
        let form = parse_topic_form(&pairs(&[
            ("group", "8"),
            ("title", "Опрос"),
            ("msg", "Текст"),
            ("tags", "lor"),
            ("variant_id", "12"),
            ("poll", "Да"),
            ("variant_id", "0"),
            ("poll", "Нет"),
        ]))
        .unwrap();
        assert_eq!(form.poll, ["Да", "Нет"]);
    }
}

pub async fn index(
    State(state): State<AppState>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let _ = q;
    let stProfileSettings = match &current_user {
        Some(user) => {
            let settings_text: Option<String> =
                sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                    .bind(user.id)
                    .fetch_optional(&state.pool)
                    .await?
                    .flatten();
            crate::profile::ProfileSettings::from_hstore_text(settings_text)
        }
        None => crate::profile::ProfileSettings::default(),
    };
    let show_gallery_on_main = stProfileSettings.main_gallery;
    let vecMainTopics = topic_service(&state)
        .vecListMainTopics(
            show_gallery_on_main,
            current_user.as_ref().map(|stUser| stUser.id),
            30,
        )
        .await?;
    let mut iFullNonMinor = 0;
    let mut vecFullTopics = Vec::new();
    let mut brief = Vec::new();
    for stMainTopic in vecMainTopics {
        if bMainTopicUsesFullCard(stMainTopic.bMinor, &mut iFullNonMinor) {
            vecFullTopics.push(stMainTopic.stTopic);
        } else {
            brief.push(stMainTopic.stTopic);
        }
    }
    let news =
        prepare_news_topics_for_viewer(&state, vecFullTopics, true, &current_user, &csrf_token)
            .await?;
    let add_restriction: i32 = if show_gallery_on_main {
        sqlx::query_scalar("SELECT COALESCE(min(restrict_topics),-9999) FROM sections")
            .fetch_one(&state.pool)
            .await?
    } else {
        sqlx::query_scalar("SELECT COALESCE(restrict_topics,-9999) FROM sections WHERE id=1")
            .fetch_one(&state.pool)
            .await?
    };
    let add_reason = posting_reason_for_port(&state, add_restriction, &current_user).await?;
    let mut uncommitted = sqlx::query_as::<_, (i32, String, i64)>(
        "SELECT s.id,s.name,count(t.id) FROM sections s JOIN groups g ON g.section=s.id JOIN topics t ON t.groupid=g.id WHERE s.moderate AND NOT t.moderate AND NOT t.deleted AND NOT t.draft AND t.postdate > (CURRENT_TIMESTAMP-'3 month'::interval) GROUP BY s.id,s.name HAVING count(t.id)>0 ORDER BY s.id",
    ).fetch_all(&state.pool).await?;
    let can_review_all_sections = current_user
        .as_ref()
        .is_some_and(|user| user.canmod || user.corrector);
    if !show_gallery_on_main && !can_review_all_sections {
        uncommitted.retain(|(section_id, _, _)| *section_id == 1);
    }
    let (drafts_count, favorite_present, user_status) = match &current_user {
        Some(user) => {
            let drafts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM topics WHERE userid=$1 AND draft AND NOT deleted",
            )
            .bind(user.id)
            .fetch_one(&state.pool)
            .await?;
            let favorites: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE userid=$1 AND watch=false)",
            )
            .bind(user.id)
            .fetch_one(&state.pool)
            .await?;
            let status = if user.score.unwrap_or(0) >= 100 {
                "активный пользователь"
            } else {
                "новый пользователь"
            };
            (drafts, favorites, status.to_string())
        }
        None => (0, false, String::new()),
    };
    let boxlets_html = StMainBoxletsTemplate {
        vecBoxlets: vecRenderMainBoxlets(
            &state,
            show_gallery_on_main,
            stProfileSettings.messages,
            current_user.as_ref().map(|stUser| stUser.id),
            &csrf_token,
        )
        .await?,
    }
    .render()?;
    Ok(Html(
        MainPageTemplate {
            news,
            brief,
            add_url: add_reason.is_none().then(|| {
                if show_gallery_on_main {
                    "/add-section.jsp".to_string()
                } else {
                    "/add-section.jsp?section=1".to_string()
                }
            }),
            add_reason: add_reason.unwrap_or_default(),
            uncommitted,
            current_user,
            user_status,
            drafts_count,
            favorite_present,
            boxlets_html,
            show_gallery_on_main,
        }
        .render()?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnForumFeedFilter {
    All,
    NoTalks,
    Tech,
}

impl EnForumFeedFilter {
    fn parse(optValue: Option<&str>) -> Result<Self> {
        match optValue {
            None => Ok(Self::All),
            Some("notalks") => Ok(Self::NoTalks),
            Some("tech") => Ok(Self::Tech),
            Some(_) => Err(AppError::BadRequest(
                "Некорректное значение filter".to_owned(),
            )),
        }
    }

    fn optId(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::NoTalks => Some("notalks"),
            Self::Tech => Some("tech"),
        }
    }

    fn optTitle(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::NoTalks => Some("без talks"),
            Self::Tech => Some("тех. форум"),
        }
    }

    fn bNoTalks(self) -> bool {
        self == Self::NoTalks
    }

    fn bTech(self) -> bool {
        self == Self::Tech
    }
}

#[derive(Debug, Deserialize)]
pub struct StForumFeedQuery {
    offset: Option<i64>,
    filter: Option<String>,
}

fn sTopicFeedPageUrl(sBase: &str, optFilter: Option<&str>, iOffset: i64) -> String {
    let mut vecParams = Vec::new();
    if let Some(sFilter) = optFilter {
        vecParams.push(format!("filter={}", urlencoding::encode(sFilter)));
    }
    if iOffset > 0 {
        vecParams.push(format!("offset={iOffset}"));
    }
    if vecParams.is_empty() {
        sBase.to_owned()
    } else {
        format!("{sBase}?{}", vecParams.join("&"))
    }
}

fn stTopicFeedLinks(
    sBase: &str,
    optFilter: Option<&str>,
    stPager: &Pager,
    iItemCount: usize,
) -> (Option<String>, Option<String>) {
    let optPrev = (stPager.offset >= stPager.limit)
        .then(|| sTopicFeedPageUrl(sBase, optFilter, (stPager.offset - stPager.limit).max(0)));
    let optNext = crate::pagination::topic_feed_has_next(stPager, iItemCount)
        .then(|| sTopicFeedPageUrl(sBase, optFilter, stPager.next_offset));
    (optPrev, optNext)
}

#[cfg(test)]
mod topic_listing_contract_tests {
    use super::{EnForumFeedFilter, stTopicFeedLinks};

    #[test]
    fn forum_filter_parser_accepts_only_java_values() {
        assert_eq!(
            EnForumFeedFilter::parse(None).unwrap(),
            EnForumFeedFilter::All
        );
        assert_eq!(
            EnForumFeedFilter::parse(Some("notalks")).unwrap(),
            EnForumFeedFilter::NoTalks
        );
        assert_eq!(
            EnForumFeedFilter::parse(Some("tech")).unwrap(),
            EnForumFeedFilter::Tech
        );
        assert!(EnForumFeedFilter::parse(Some("")).is_err());
        assert!(EnForumFeedFilter::parse(Some("all")).is_err());
        assert!(EnForumFeedFilter::parse(Some("TECH")).is_err());
    }

    #[test]
    fn forum_pager_preserves_filter_in_both_directions() {
        let stPager = crate::pagination::topic_feed_pager(20);
        let (optPrev, optNext) = stTopicFeedLinks("/forum/lenta", Some("notalks"), &stPager, 20);

        assert_eq!(optPrev.as_deref(), Some("/forum/lenta?filter=notalks"));
        assert_eq!(
            optNext.as_deref(),
            Some("/forum/lenta?filter=notalks&offset=40")
        );
    }

    #[test]
    fn partial_topic_page_has_no_next_link() {
        let stPager = crate::pagination::topic_feed_pager(0);
        let (_, optNext) = stTopicFeedLinks("/news/", None, &stPager, 19);

        assert!(optNext.is_none());
    }
}

pub async fn lenta(
    State(state): State<AppState>,
    Query(q): Query<StForumFeedQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let enFilter = EnForumFeedFilter::parse(q.filter.as_deref())?;
    let pager = crate::pagination::topic_feed_pager(q.offset.unwrap_or(0));
    let topics = list_topics_filtered(
        &state,
        Some("forum"),
        None,
        pager.offset,
        pager.limit,
        enFilter.bNoTalks(),
        enFilter.bTech(),
    )
    .await?;
    let (prev_link, next_link) =
        stTopicFeedLinks("/forum/lenta", enFilter.optId(), &pager, topics.len());
    let news =
        prepare_news_topics_for_viewer(&state, topics.clone(), true, &current_user, &csrf_token)
            .await?;
    let mut navigation = build_topic_list_navigation(&state, "forum", None, &current_user).await?;
    navigation.section_url = None;
    navigation.quick_groups.clear();
    navigation.rss_url = Some(match enFilter.optId() {
        Some(sFilter) => format!("/section-rss.jsp?section=2&filter={sFilter}"),
        None => "/section-rss.jsp?section=2".to_owned(),
    });
    navigation.forum_filters = vec![
        ForumFilterLink {
            label: "все",
            url: "/forum/lenta",
            selected: enFilter == EnForumFeedFilter::All,
        },
        ForumFilterLink {
            label: "без talks",
            url: "/forum/lenta?filter=notalks",
            selected: enFilter == EnForumFeedFilter::NoTalks,
        },
        ForumFilterLink {
            label: "тех. форум",
            url: "/forum/lenta?filter=tech",
            selected: enFilter == EnForumFeedFilter::Tech,
        },
    ];
    let title = match enFilter.optTitle() {
        Some(sFilterTitle) => format!("Форум ({sFilterTitle})"),
        None => "Форум".to_owned(),
    };
    Ok(Html(
        IndexTemplate {
            title,
            topics,
            news,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

pub async fn section_topics(
    State(state): State<AppState>,
    uri: Uri,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = crate::pagination::topic_feed_pager(q.offset.unwrap_or(0));
    let topics = list_topics(&state, Some(section), None, pager.offset, pager.limit).await?;
    let (prev_link, next_link) =
        stTopicFeedLinks(&format!("/{section}/"), None, &pager, topics.len());
    let news =
        prepare_news_topics_for_viewer(&state, topics.clone(), true, &current_user, &csrf_token)
            .await?;
    let navigation = build_topic_list_navigation(&state, section, None, &current_user).await?;
    Ok(Html(
        IndexTemplate {
            title: section_title(section).to_string(),
            topics,
            news,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

pub async fn section_group_topics(
    State(state): State<AppState>,
    uri: Uri,
    Path(group): Path<String>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = crate::pagination::topic_feed_pager(q.offset.unwrap_or(0));
    let topics = list_topics(
        &state,
        Some(section),
        Some(&group),
        pager.offset,
        pager.limit,
    )
    .await?;
    let selected = crate::routes::groups::find_group_by_section(&state, section, &group).await?;
    let (prev_link, next_link) = stTopicFeedLinks(
        &format!("/{section}/{}", urlencoding::encode(&selected.urlname)),
        None,
        &pager,
        topics.len(),
    );
    let news =
        prepare_news_topics_for_viewer(&state, topics.clone(), false, &current_user, &csrf_token)
            .await?;
    let navigation =
        build_topic_list_navigation(&state, section, Some(&selected), &current_user).await?;
    Ok(Html(
        IndexTemplate {
            title: format!("{} «{}»", section_title(section), selected.title),
            topics,
            news,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct LegacyShowTopicsQuery {
    pub nick: Option<String>,
    pub output: Option<String>,
}

fn stLegacyShowTopicsRedirect(stQuery: LegacyShowTopicsQuery) -> Result<Response> {
    // TopicListController.showUserTopics binds `nick` with @RequestParam.
    // Spring rejects an omitted required parameter with HTTP 400 before the
    // controller runs (confirmed against the original runtime).
    let sNick = stQuery
        .nick
        .ok_or_else(|| AppError::BadRequest("Required parameter 'nick' is missing".to_owned()))?;
    let sNick = urlencoding::encode(&sNick);
    let sLocation = if stQuery.output.is_some() {
        // Presence, not the value, selects the retired RSS branch.
        format!("/people/{sNick}/?output=rss")
    } else {
        format!("/people/{sNick}/")
    };

    // Spring RedirectView uses 302 for this legacy GET endpoint.  Axum's
    // Redirect::to is 303, so construct the response explicitly.
    Ok((StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response())
}

pub async fn legacy_show_topics(Query(stQuery): Query<LegacyShowTopicsQuery>) -> Result<Response> {
    stLegacyShowTopicsRedirect(stQuery)
}

#[cfg(test)]
mod legacy_show_topics_tests {
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };

    use super::{LegacyShowTopicsQuery, stLegacyShowTopicsRedirect};
    use crate::error::AppError;

    #[test]
    fn redirects_to_the_canonical_user_topic_list_with_java_302() {
        let stResponse = stLegacyShowTopicsRedirect(LegacyShowTopicsQuery {
            nick: Some("maxcom".to_owned()),
            output: None,
        })
        .expect("legacy redirect");

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/people/maxcom/")
        );
    }

    #[test]
    fn any_output_value_selects_the_retired_rss_redirect() {
        let stResponse = stLegacyShowTopicsRedirect(LegacyShowTopicsQuery {
            nick: Some("maxcom".to_owned()),
            output: Some("atom".to_owned()),
        })
        .expect("legacy RSS redirect");

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/people/maxcom/?output=rss")
        );
    }

    #[test]
    fn missing_nick_uses_the_original_spring_binding_400_contract() {
        let stError = stLegacyShowTopicsRedirect(LegacyShowTopicsQuery {
            nick: None,
            output: None,
        })
        .expect_err("nick is required");

        assert!(matches!(stError, AppError::BadRequest(_)));
        assert_eq!(stError.into_response().status(), StatusCode::BAD_REQUEST);
    }
}

const VIEW_ALL_SECTION_PREFIX_CASE: &str = "CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END";
const GALLERY_SECTION_ID: i32 = 3;

#[derive(Debug, Clone, sqlx::FromRow)]
struct ViewAllSection {
    id: i32,
    name: String,
    restrict_score: i32,
    section_prefix: String,
}

impl ViewAllSection {
    /// Section.uncommitedName
    fn uncommited_name(&self) -> String {
        if self.id == GALLERY_SECTION_ID {
            "Неподтверждённые галереи".to_string()
        } else {
            format!("Неподтверждённые {}", self.name.to_lowercase())
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct DeletedTopicRow {
    subj: String,
    nick: String,
    msgid: i32,
    reason: Option<String>,
    postdate: chrono::DateTime<chrono::Utc>,
    deldate: Option<chrono::DateTime<chrono::Utc>>,
    bonus: Option<i32>,
}

impl DeletedTopicRow {
    fn reason_display(&self) -> &str {
        self.reason.as_deref().unwrap_or_default()
    }

    fn sTitlePlain(&self) -> String {
        crate::domain::title::sPlainForDisplay(&self.subj)
    }
}

#[derive(Template)]
#[template(path = "view_all.html")]
struct ViewAllTemplate {
    title: String,
    section: Option<ViewAllSection>,
    uncommitted_counts: Vec<(ViewAllSection, i64)>,
    uncommitted: i64,
    add_link: Option<String>,
    add_link_reason: Option<String>,
    messages: Vec<NewsTopicView>,
    deleted_topics: Vec<DeletedTopicRow>,
    show_dates: bool,
    show_gallery_notice: bool,
}

#[derive(Deserialize)]
pub struct ViewAllQuery {
    pub section: Option<i32>,
}

const POSTSCORE_UNRESTRICTED: i32 = -9999;
const POSTSCORE_REGISTERED_ONLY: i32 = -50;
const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
const POSTSCORE_NO_COMMENTS: i32 = 10001;
const POSTSCORE_HIDE_COMMENTS: i32 = 10002;

/// Request-IP-independent AddTopicChecker hint used by topic-list navigation.
/// The actual `/add.jsp` GET/POST enforcement goes through
/// `CAddTopicService`, including frozen-user and canonical `b_ips` checks.
pub(crate) fn topic_posting_reason(restriction: i32, user: &Option<UserSummary>) -> Option<String> {
    let anonymous = user.is_none();
    let score = user.as_ref().and_then(|u| u.score).unwrap_or(0);
    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    match restriction {
        POSTSCORE_UNRESTRICTED => None,
        POSTSCORE_MODERATORS_ONLY => {
            if is_moderator {
                None
            } else {
                Some("только для модераторов".to_string())
            }
        }
        POSTSCORE_REGISTERED_ONLY => {
            if anonymous {
                Some("только для зарегистрированных".to_string())
            } else {
                None
            }
        }
        POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS => Some("постинг запрещен".to_string()),
        _ => {
            if anonymous || score < restriction {
                Some(format!(
                    "только для зарегистрированных, score>={restriction}"
                ))
            } else {
                None
            }
        }
    }
}

/// Navigation-level posting hint. Anonymous posting is a real Java workflow:
/// unrestricted groups must expose the add link even without a session. The
/// request-IP checks remain in the `/add.jsp` handler where the client address
/// is available.
pub(crate) async fn posting_reason_for_port(
    state: &AppState,
    restriction: i32,
    user: &Option<UserSummary>,
) -> Result<Option<String>> {
    if let Some(current) = user {
        if current.blocked.unwrap_or(false) {
            return Ok(Some("аккаунт заблокирован".to_string()));
        }
        let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
                .bind(current.id)
                .fetch_optional(&state.pool)
                .await?
                .flatten();
        if frozen_until.is_some_and(|until| until > chrono::Utc::now()) {
            return Ok(Some("аккаунт заморожен".to_string()));
        }
    }
    Ok(topic_posting_reason(restriction, user))
}

pub(crate) async fn build_topic_list_navigation(
    state: &AppState,
    section_prefix: &str,
    selected_group: Option<&Group>,
    user: &Option<UserSummary>,
) -> Result<TopicListNavigation> {
    let (section_id, section_restriction, section_premoderated): (i32, i32, bool) = sqlx::query_as(
        r#"SELECT id, COALESCE(restrict_topics,-9999), moderate FROM sections WHERE CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END=$1"#,
    )
    .bind(section_prefix)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let groups = crate::routes::groups::list_groups_by_section(state, Some(section_prefix)).await?;
    let restriction = if let Some(group) = selected_group {
        let group_restriction: i32 =
            sqlx::query_scalar("SELECT COALESCE(restrict_topics, -9999) FROM groups WHERE id=$1")
                .bind(group.id)
                .fetch_one(&state.pool)
                .await?;
        section_restriction.max(group_restriction)
    } else {
        section_restriction
    };
    let add_reason = posting_reason_for_port(state, restriction, user).await?;
    let add_url = add_reason.is_none().then(|| match selected_group {
        Some(group) => format!("/add.jsp?group={}", group.id),
        None => format!("/add-section.jsp?section={section_id}"),
    });
    let quick_groups = groups
        .into_iter()
        .map(|group| QuickGroupLink {
            title: group.title,
            url: format!("/{section_prefix}/{}", group.urlname),
            selected: selected_group.is_some_and(|selected| selected.id == group.id),
        })
        .collect();
    let uncommitted_count = if section_premoderated {
        sqlx::query_scalar(
            r#"SELECT count(*) FROM topics t
               JOIN groups g ON g.id=t.groupid
               WHERE g.section=$1 AND NOT t.moderate AND NOT t.deleted AND NOT t.draft
                 AND t.postdate>CURRENT_TIMESTAMP-interval '3 months'"#,
        )
        .bind(section_id)
        .fetch_one(&state.pool)
        .await?
    } else {
        0
    };
    let active_tags = if selected_group.is_some_and(|stGroup| stGroup.id == 4068) {
        Vec::new()
    } else {
        let optGroup = selected_group.map(|stGroup| stGroup.urlname.as_str());
        match tokio::time::timeout(
            std::time::Duration::from_millis(500),
            crate::search_index::vecActiveTopTags(state, section_prefix, optGroup),
        )
        .await
        {
            Ok(Ok(vecTags)) => vecTags
                .into_iter()
                .map(|sName| ActiveTagLink {
                    url: format!("/tag/{}?section={section_id}", urlencoding::encode(&sName)),
                    name: sName,
                })
                .collect(),
            Ok(Err(sError)) => {
                tracing::warn!(error = %sError, section = section_prefix, "unable to find active section tags");
                Vec::new()
            }
            Err(_) => {
                tracing::warn!(
                    section = section_prefix,
                    "active section tags search timed out"
                );
                Vec::new()
            }
        }
    };
    Ok(TopicListNavigation {
        section_id,
        section_url: Some(format!("/{section_prefix}/")),
        archive_url: (section_prefix != "forum").then(|| format!("/{section_prefix}/archive")),
        rss_url: Some(format!("/section-rss.jsp?section={section_id}")),
        add_url,
        add_reason: add_reason.unwrap_or_default(),
        moderator_group_id: user
            .as_ref()
            .is_some_and(|u| u.canmod)
            .then(|| selected_group.map(|g| g.id))
            .flatten(),
        quick_groups,
        all_groups_selected: selected_group.is_none(),
        uncommitted_count,
        active_tags,
        forum_filters: Vec::new(),
    })
}

/// UncommitedTopicsController/view-all.jsp: the premoderation queue -
/// public (no auth required, matching Java's `MaybeAuthorized`), lists
/// topics awaiting commit in premoderated sections plus recently deleted
/// ones, with an add-topic shortcut gated on posting permission.
pub async fn view_all(
    State(state): State<AppState>,
    Query(q): Query<ViewAllQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let section: Option<ViewAllSection> = if let Some(sid) = q.section.filter(|&id| id != 0) {
        let sql = format!(
            "SELECT s.id, s.name, COALESCE(s.restrict_topics,-9999) AS restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix FROM sections s WHERE s.id=$1"
        );
        Some(
            sqlx::query_as::<_, ViewAllSection>(sqlx::AssertSqlSafe(sql))
                .bind(sid)
                .fetch_optional(&state.pool)
                .await?
                .ok_or(AppError::NotFound)?,
        )
    } else {
        None
    };

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE NOT t.deleted AND NOT t.draft AND NOT t.moderate AND s.moderate
             AND t.postdate >= now() - interval '3 months'
             AND ($1::int IS NULL OR s.id=$1)
           ORDER BY t.postdate DESC"#
    );
    let message_topics = sqlx::query_as::<_, TopicSummary>(sqlx::AssertSqlSafe(sql))
        .bind(section.as_ref().map(|s| s.id))
        .fetch_all(&state.pool)
        .await?;
    let uncommitted = message_topics.len() as i64;
    let mut messages =
        prepare_news_topics_for_viewer(&state, message_topics, true, &user, &csrf_token).await?;
    for message in &mut messages {
        message.moderate_mode = true;
        if let Some(current_user) = &user {
            message.can_commit =
                check_commit_allowed(current_user, message.topic.author_id).is_ok();
            // Every item in this list is a recent, non-deleted topic from a
            // premoderated section.  These are the resulting TopicMenu rules
            // from EditTopicChecker/GroupPermissionService for that state.
            message.can_edit = current_user.candel
                || current_user.canmod
                || current_user.corrector
                || current_user.id == message.topic.author_id;
            message.can_delete = current_user.candel
                || current_user.canmod
                || (current_user.id == message.topic.author_id
                    && message.topic.comments == 0
                    && chrono::Utc::now()
                        <= message.topic.postdate
                            + chrono::Duration::hours(TOPIC_DELETE_WINDOW_HOURS));
        }
    }

    let bad_reason_filter = if is_moderator {
        ""
    } else {
        "AND di.reason != '' AND di.reason NOT IN ('Блокировка пользователя с удалением сообщений','4.6 Спам')"
    };
    let sql = format!(
        r#"SELECT t.title AS subj, u.nick, t.id AS msgid, di.reason, t.postdate, di.deldate, di.bonus
           FROM topics t, groups g, users u, sections s, del_info di
           WHERE s.id=g.section AND t.userid=u.id AND t.groupid=g.id AND s.moderate AND t.deleted
             AND di.msgid=t.id AND t.userid != di.delby
             AND di.deldate > now() - interval '2 weeks'
             AND ($1::int IS NULL OR s.id=$1)
             {bad_reason_filter}
           ORDER BY di.deldate DESC LIMIT 20"#
    );
    let deleted_topics = sqlx::query_as::<_, DeletedTopicRow>(sqlx::AssertSqlSafe(sql))
        .bind(section.as_ref().map(|s| s.id))
        .fetch_all(&state.pool)
        .await?;

    let uncommitted_counts: Vec<(ViewAllSection, i64)> = if section.is_none() {
        let sql = format!(
            r#"SELECT s.id, s.name, COALESCE(s.restrict_topics,-9999) AS restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix, count(t.*) AS cnt
               FROM sections s
               JOIN groups g ON g.section=s.id
               JOIN topics t ON t.groupid=g.id
               WHERE s.moderate AND NOT t.draft AND NOT t.deleted AND NOT t.moderate
                 AND t.postdate >= now() - interval '3 months'
               GROUP BY s.id
               ORDER BY s.id"#
        );
        sqlx::query_as::<_, (i32, String, i32, String, i64)>(sqlx::AssertSqlSafe(sql))
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(|(id, name, restrict_score, section_prefix, cnt)| {
                (
                    ViewAllSection {
                        id,
                        name,
                        restrict_score,
                        section_prefix,
                    },
                    cnt,
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let restriction = match &section {
        Some(s) => s.restrict_score,
        None => {
            sqlx::query_scalar::<_, i32>(
                "SELECT COALESCE(min(restrict_topics),-9999) FROM sections",
            )
            .fetch_one(&state.pool)
            .await?
        }
    };
    let posting_reason = posting_reason_for_port(&state, restriction, &user).await?;
    let (add_link, add_link_reason) = match posting_reason {
        None => (
            Some(match &section {
                Some(s) => format!("/add-section.jsp?section={}", s.id),
                None => "/add-section.jsp".to_string(),
            }),
            None,
        ),
        Some(reason) => (None, Some(reason)),
    };

    let title = section
        .as_ref()
        .map(|s| s.uncommited_name())
        .unwrap_or_else(|| "Просмотр неподтверждённых сообщений".to_string());
    let show_gallery_notice = section
        .as_ref()
        .map(|s| s.id == GALLERY_SECTION_ID)
        .unwrap_or(true);

    Ok(Html(
        ViewAllTemplate {
            title,
            section,
            uncommitted_counts,
            uncommitted,
            add_link,
            add_link_reason,
            messages,
            deleted_topics,
            show_dates: is_moderator,
            show_gallery_notice,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct ViewMessageQuery {
    msgid: i32,
    #[serde(rename = "fromHistory")]
    from_history: Option<i32>,
    page: Option<i32>,
    lastmod: Option<i64>,
    filter: Option<String>,
    output: Option<String>,
}

pub async fn legacy_view_message(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
) -> Result<Response> {
    let topic = get_topic(&state, q.msgid).await?;
    let mut target = topic.topic_url();
    if let Some(page) = q.page {
        target.push_str(&format!("/page{page}"));
    }
    let mut params = Vec::new();
    if q.lastmod.is_some() {
        let expired: bool = sqlx::query_scalar(
            r#"SELECT NOT t.sticky
                      AND COALESCE(t.commitdate,t.postdate) < CURRENT_TIMESTAMP-s.expire
                 FROM topics t
                 JOIN groups g ON g.id=t.groupid
                 JOIN sections s ON s.id=g.section
                WHERE t.id=$1"#,
        )
        .bind(topic.id)
        .fetch_one(&state.pool)
        .await?;
        if !expired && let Some(lastmod) = topic.lastmod {
            params.push(format!("lastmod={}", lastmod.timestamp_millis()));
        }
    }
    if let Some(filter) = q.filter {
        params.push(format!("filter={}", urlencoding::encode(&filter)));
    }
    if let Some(output) = q.output {
        params.push(format!("output={}", urlencoding::encode(&output)));
    }
    if !params.is_empty() {
        target.push('?');
        target.push_str(&params.join("&"));
    }
    Ok((StatusCode::FOUND, [(header::LOCATION, target)]).into_response())
}

#[derive(Deserialize, Default)]
pub struct TopicViewQuery {
    pub cid: Option<i32>,
    /// Presence-based, like Java's `deleted != null` - any non-empty
    /// query string (`?deleted` or `?deleted=true`) requests the
    /// moderator-only deleted-comments view.
    pub deleted: Option<String>,
    /// "show" disables ignore-list-based comment hiding for this request.
    pub filter: Option<String>,
    pub results: Option<bool>,
}

pub async fn topic_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: Uri,
    Path((group, id)): Path<(String, i32)>,
    Query(q): Query<TopicViewQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    render_topic_view(
        state,
        section,
        group,
        id,
        0,
        None,
        q,
        current_user,
        csrf_token,
        sRemoteIp,
    )
    .await
}

pub async fn topic_page_with_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: Uri,
    Path((group, id, page_marker)): Path<(String, i32, String)>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let Some(page) = page_marker.strip_prefix("page") else {
        return Err(AppError::NotFound);
    };
    let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
    let section = section_from_uri(&uri).unwrap_or("forum");
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    // Java's getMessagePage doesn't accept `cid`/`deleted`/`filter` at all -
    // only the base (page-less) route does.
    render_topic_view(
        state,
        section,
        group,
        id,
        page,
        None,
        TopicViewQuery::default(),
        current_user,
        csrf_token,
        sRemoteIp,
    )
    .await
}

pub async fn topic_thread(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: Uri,
    Path((group, id, thread_root)): Path<(String, i32, i32)>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    render_topic_view(
        state,
        section,
        group,
        id,
        0,
        Some(thread_root),
        TopicViewQuery::default(),
        current_user,
        csrf_token,
        sRemoteIp,
    )
    .await
}

/// Called from legacy.rs's combined `/forum/{group}/{id_or_year}/{page_or_month}`
/// route once it's determined the third segment is `pageN`, not a year/month.
pub async fn render_topic_page(
    state: AppState,
    section: &'static str,
    group: String,
    id: i32,
    page: i64,
    current_user: Option<UserSummary>,
    csrf_token: String,
    sRemoteIp: String,
) -> Result<Response> {
    render_topic_view(
        state,
        section,
        group,
        id,
        page,
        None,
        TopicViewQuery::default(),
        current_user,
        csrf_token,
        sRemoteIp,
    )
    .await
}

pub(crate) async fn messages_per_page(state: &AppState, user: &Option<UserSummary>) -> i64 {
    match user {
        Some(u) => {
            let settings_text: Option<String> =
                sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                    .bind(u.id)
                    .fetch_optional(&state.pool)
                    .await
                    .ok()
                    .flatten();
            crate::profile::ProfileSettings::from_hstore_text(settings_text).messages as i64
        }
        None => crate::profile::DEFAULT_MESSAGES as i64,
    }
}

/// TopicController.getMessageMain/getMessagePage/getMessageThread, merged
/// into one function parameterized by page/thread_root/query - mirrors
/// Java's shared private `getMessage` helper.
async fn render_topic_view(
    state: AppState,
    section: &'static str,
    group: String,
    id: i32,
    page: i64,
    thread_root: Option<i32>,
    query: TopicViewQuery,
    current_user: Option<UserSummary>,
    csrf_token: String,
    sRemoteIp: String,
) -> Result<Response> {
    let topic = get_topic(&state, id).await?;
    let is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);

    // TopicPermissionService.checkView restricts drafts and deleted topics,
    // but deliberately does not hide an uncommitted topic by itself.  The
    // public `/view-all.jsp` queue therefore links to a publicly viewable
    // preview, matching the Java application.
    if topic.draft || topic.deleted {
        let allowed = current_user
            .as_ref()
            .map(|u| u.canmod || u.id == topic.author_id)
            .unwrap_or(false);
        if !allowed {
            return Err(AppError::NotFound);
        }
    }

    // TopicController.getMessageMain: canonical redirect if the URL's
    // group/section don't match the topic's real ones.
    if topic.group_urlname != group || topic.section_prefix != section {
        return Ok(Redirect::to(&topic.topic_url()).into_response());
    }

    let want_deleted = query.deleted.is_some();
    let can_view_deleted_comments =
        allow_view_all_deleted_comments(&state, topic.id, &current_user).await?;
    if want_deleted && !can_view_deleted_comments {
        return Ok(Redirect::to(&topic.topic_url()).into_response());
    }

    // `?cid=` jumps straight to the comment (resolving its page), bypassing
    // the rest of rendering entirely - matches Java's inline jumpMessage
    // short-circuit in getMessageMain. Only the base (page-less, non-thread)
    // route wires this in.
    if let Some(cid) = query.cid
        && thread_root.is_none()
        && page == 0
    {
        return resolve_comment_jump(&state, &topic, cid, is_moderator, &current_user).await;
    }

    // TopicController starts MoreLikeThis immediately and gives the JSP only
    // the remainder of a 500 ms deadline after the main page work. Keep the
    // OpenSearch request running after a page-timeout so it can populate the
    // one-hour cache for the next view, matching the original async service.
    let stSimilarStarted = std::time::Instant::now();
    let stSimilarState = state.clone();
    let iSimilarTopicId = topic.id;
    let sSimilarTitle = html_escape::decode_html_entities(&topic.title).into_owned();
    let vecSimilarTags = topic.tags_vec();
    let stSimilarTask = tokio::spawn(async move {
        crate::search_index::vecSimilarTopics(
            &stSimilarState,
            iSimilarTopicId,
            &sSimilarTitle,
            &vecSimilarTags,
        )
        .await
    });

    let all_comments: Vec<CommentItem> = if want_deleted {
        topic_service(&state).vecListComments(id).await?
    } else {
        topic_service(&state)
            .vecListComments(id)
            .await?
            .into_iter()
            .filter(|c| !c.deleted)
            .collect()
    };
    let iRealtimeLastCommentId = all_comments.last().map_or(0, |stComment| stComment.id);
    let setCommentsWithReplies: std::collections::HashSet<i32> = all_comments
        .iter()
        .filter_map(|stComment| stComment.replyto)
        .collect();

    // TopicController's hideSet: comments from ignored authors are dropped
    // from the rendered list (not just visually) unless `?filter=show`.
    let filter_show = query.filter.as_deref() == Some("show");
    let ignored_ids: Vec<i32> = match (&current_user, filter_show) {
        (Some(u), false) => sqlx::query_scalar("SELECT ignored FROM ignore_list WHERE userid=$1")
            .bind(u.id)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default(),
        _ => vec![],
    };
    let unfiltered_count = all_comments.len();
    let visible_comments: Vec<CommentItem> = if ignored_ids.is_empty() {
        all_comments
    } else {
        all_comments
            .into_iter()
            .filter(|c| !ignored_ids.contains(&c.author_id))
            .collect()
    };
    let filtered_count = visible_comments.len();
    let mapCommentReplies: std::collections::HashMap<i32, CommentReplyView> = visible_comments
        .iter()
        .map(|stComment| {
            (
                stComment.id,
                CommentReplyView {
                    id: stComment.id,
                    title: stComment.optTitlePlain(),
                    author: stComment.author.clone(),
                    postdate: stComment.postdate,
                },
            )
        })
        .collect();
    let mut mapCommentAnswers: std::collections::HashMap<i32, Vec<i32>> =
        std::collections::HashMap::new();
    for stComment in &visible_comments {
        if let Some(iReplyTo) = stComment.replyto {
            mapCommentAnswers
                .entry(iReplyTo)
                .or_default()
                .push(stComment.id);
        }
    }

    let (page_comments, pages, _thread_mode, bHasNextPage): (
        Vec<CommentItem>,
        Vec<CommentPageLink>,
        bool,
        bool,
    ) = if let Some(root) = thread_root {
        let subtree = comment_subtree(&visible_comments, root);
        (subtree, vec![], true, false)
    } else if want_deleted {
        // Java's showDeleted path uses page=-1: render every comment on one
        // page, no pagination controls.
        (visible_comments, vec![], false, false)
    } else {
        let per_page = messages_per_page(&state, &current_user).await.max(1);
        let total_pages = (unfiltered_count as i64 + per_page - 1) / per_page.max(1);
        if page > 0 && page >= total_pages {
            let target_page = (total_pages - 1).max(0);
            let url = if target_page > 0 {
                format!("{}/page{target_page}", topic.topic_url())
            } else {
                topic.topic_url()
            };
            return Ok(Redirect::to(&url).into_response());
        }
        let start = (page * per_page) as usize;
        let end = (start + per_page as usize).min(visible_comments.len());
        let slice = if start < visible_comments.len() {
            visible_comments[start..end].to_vec()
        } else {
            vec![]
        };
        let pages = if total_pages > 1 {
            (0..total_pages)
                .map(|p| CommentPageLink {
                    page: p,
                    current: p == page,
                })
                .collect()
        } else {
            vec![]
        };
        (slice, pages, false, page + 1 < total_pages)
    };
    let stMarkupUsers = state
        .markup
        .stResolveBatch(
            std::iter::once((topic.message.as_str(), topic.markup.as_str())).chain(
                page_comments
                    .iter()
                    .map(|stComment| (stComment.message.as_str(), stComment.markup.as_str())),
            ),
        )
        .await?;
    let topic_html = markup::render_topic_with_expanded_cut_policy_and_users(
        &topic.message,
        &topic.markup,
        topic.bNofollowAuthorLinks(),
        Some(&state.config.public_url),
        Some(&stMarkupUsers),
    );

    // ReactionService.allowInteract's expired/comments-hidden/frozen inputs,
    // fetched once up front so per-comment widgets don't each hit the DB.
    let (topic_expired, topic_postscore): (bool, i32) = sqlx::query_as(
        "SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, COALESCE(t.postscore, -9999) FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1",
    )
    .bind(topic.id)
    .fetch_one(&state.pool)
    .await?;
    let comments_hidden = topic_postscore == POSTSCORE_HIDE_COMMENTS;
    let reactor_frozen = match &current_user {
        Some(u) => sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT frozen_until FROM users WHERE id=$1",
        )
        .bind(u.id)
        .fetch_one(&state.pool)
        .await?
        .map(|t| t > chrono::Utc::now())
        .unwrap_or(false),
        None => false,
    };
    let all_reactions = load_all_reactions(
        &state,
        topic.id,
        current_user.as_ref().map(|stUser| stUser.id),
    )
    .await?;
    let current_user_id = current_user.as_ref().map(|u| u.id);

    let vecPageCommentIds = page_comments
        .iter()
        .map(|stComment| stComment.id)
        .collect::<Vec<_>>();
    let vecDeleteInfoRows: Vec<(i32, i32, String, String)> = if vecPageCommentIds.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            r#"SELECT di.msgid, di.delby, u.nick, COALESCE(di.reason,'')
               FROM del_info di JOIN users u ON u.id=di.delby
               WHERE di.msgid=ANY($1)"#,
        )
        .bind(&vecPageCommentIds)
        .fetch_all(&state.pool)
        .await?
    };
    let mapDeleteInfo = vecDeleteInfoRows
        .into_iter()
        .map(|(iCommentId, iUserId, sNick, sReason)| {
            (
                iCommentId,
                CommentDeleteInfoView {
                    user_id: iUserId,
                    nick: sNick,
                    reason: sReason,
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    let bModeratorSession = current_user.as_ref().is_some_and(|stUser| stUser.canmod);
    let mut vecAuthorIds = Vec::with_capacity(page_comments.len() + 1);
    vecAuthorIds.push(topic.author_id);
    vecAuthorIds.extend(page_comments.iter().map(|stComment| stComment.author_id));
    vecAuthorIds.sort_unstable();
    vecAuthorIds.dedup();
    let vecSignatureRows: Vec<TyAuthorPresentationRow> = sqlx::query_as(
        r#"SELECT id, COALESCE(score,0), COALESCE(max_score,0), COALESCE(passwd,'')<>'',
                  photo, email
           FROM users WHERE id=ANY($1)"#,
    )
    .bind(&vecAuthorIds)
    .fetch_all(&state.pool)
    .await?;
    let mapAuthorSignatures: std::collections::HashMap<i32, AuthorSignatureView> = vecSignatureRows
        .iter()
        .map(|(iUserId, iScore, iMaxScore, bRegistered, _, _)| {
            (
                *iUserId,
                stAuthorSignature(*iScore, *iMaxScore, *bRegistered, bModeratorSession),
            )
        })
        .collect();
    let stViewerProfile = match current_user.as_ref() {
        Some(stUser) => {
            let optSettings: Option<String> =
                sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                    .bind(stUser.id)
                    .fetch_optional(&state.pool)
                    .await?;
            crate::profile::ProfileSettings::from_hstore_text(optSettings)
        }
        None => crate::profile::ProfileSettings::default(),
    };
    let mapAuthorUserpics: std::collections::HashMap<i32, (String, i32, i32)> =
        if stViewerProfile.photos {
            vecSignatureRows
                .iter()
                .map(|(iUserId, _, _, _, optPhoto, optEmail)| {
                    let optUrl = crate::profile::userpic_url(
                        &stViewerProfile.avatar,
                        false,
                        *iUserId == 2,
                        optPhoto.as_deref(),
                        optEmail.as_deref(),
                    );
                    let bDisabled = optUrl.is_none();
                    (
                        *iUserId,
                        (
                            optUrl.unwrap_or_else(|| crate::profile::DISABLED_USERPIC.to_owned()),
                            if bDisabled { 1 } else { 150 },
                            if bDisabled { 1 } else { 150 },
                        ),
                    )
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };
    let topic_author_signature = mapAuthorSignatures
        .get(&topic.author_id)
        .cloned()
        .unwrap_or_default();

    // CommentPrepareService computes this for the author of every comment,
    // not for the current viewer. The original inline form uses it to warn
    // before replying to an author who cannot continue the conversation.
    // A temporary freeze is deliberately ignored here, while a block and
    // all topic/group/section score restrictions still apply.
    let stCommentPostingContext =
        crate::routes::comments::stCommentPostingContext(&state, topic.id).await?;
    let vecAuthorUsers: Vec<UserSummary> = sqlx::query_as(
        r#"SELECT id,nick,name,score,max_score,photo,town,regdate,canmod,
                  COALESCE(candel,false) AS candel,
                  COALESCE(corrector,false) AS corrector,blocked,userinfo
           FROM users WHERE id=ANY($1)"#,
    )
    .bind(&vecAuthorIds)
    .fetch_all(&state.pool)
    .await?;
    let mapAuthorReadonly: std::collections::HashMap<i32, bool> = vecAuthorUsers
        .iter()
        .map(|stAuthor| {
            (
                stAuthor.id,
                crate::routes::comments::check_comment_posting_context(
                    &stCommentPostingContext,
                    stAuthor,
                    stAuthor.id == crate::routes::comments::ANONYMOUS_USER_ID,
                    false,
                    true,
                )
                .is_err(),
            )
        })
        .collect();

    let comments: Vec<CommentView> = page_comments
        .into_iter()
        .map(|item| {
            let optReply = item
                .replyto
                .and_then(|iReplyTo| mapCommentReplies.get(&iReplyTo).cloned());
            let vecAnswers = mapCommentAnswers.get(&item.id).cloned().unwrap_or_default();
            let iAnswerCount = vecAnswers.len();
            let sAnswerUrl = if iAnswerCount == 1 {
                format!("{}?cid={}", topic.topic_url(), vecAnswers[0])
            } else {
                format!("{}/thread/{}#comments", topic.topic_url(), item.id)
            };
            let (optUserpicUrl, iUserpicWidth, iUserpicHeight) = mapAuthorUserpics
                .get(&item.author_id)
                .map(|(sUrl, iWidth, iHeight)| (Some(sUrl.clone()), *iWidth, *iHeight))
                .unwrap_or((None, 0, 0));
            let html = markup::render_message_with_markup_policy_and_users(
                &item.message,
                Some(&item.markup),
                None,
                item.bNofollowAuthorLinks(),
                Some(&state.config.public_url),
                Some(&stMarkupUsers),
            );
            let rows: Vec<(String, i32, String, i32)> = all_reactions
                .iter()
                .filter(|(cid, ..)| *cid == Some(item.id))
                .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
                .collect();
            let allow_interact = reactions_allow_interact(
                &current_user,
                reactor_frozen,
                topic_expired,
                item.author_id,
                item.deleted,
                comments_hidden,
            );
            let reactions = render_reactions_widget(
                topic.id,
                Some(item.id),
                &rows,
                current_user_id,
                allow_interact,
                &csrf_token,
            );
            let can_edit = current_user.as_ref().is_some_and(|stUser| {
                stUser.id == item.author_id
                    && stUser.score.unwrap_or(0) >= 45
                    && (!matches!(item.markup.as_str(), "PLAIN") || stUser.candel)
                    && !item.deleted
                    && !topic_expired
                    && !setCommentsWithReplies.contains(&item.id)
                    && chrono::Utc::now() <= item.postdate + chrono::Duration::minutes(30)
            });
            let can_delete = current_user.as_ref().is_some_and(|stUser| {
                !item.deleted
                    && !topic.deleted
                    && (stUser.canmod
                        || (stUser.id == item.author_id
                            && !topic_expired
                            && !setCommentsWithReplies.contains(&item.id)
                            && chrono::Utc::now() < item.postdate + chrono::Duration::hours(3)))
            });
            let can_undelete = current_user.as_ref().is_some_and(|stUser| {
                stUser.canmod
                    && item.deleted
                    && !topic.deleted
                    && !topic_expired
                    && mapDeleteInfo
                        .get(&item.id)
                        .is_some_and(|stInfo| stInfo.user_id != item.author_id)
            });
            let can_warn = current_user.as_ref().is_some_and(|stUser| {
                stUser.score.unwrap_or(0) >= 50
                    && !reactor_frozen
                    && !topic.deleted
                    && !topic_expired
                    && !topic.draft
                    && !item.deleted
            });
            let is_topic_author = item.author_id == topic.author_id
                && item.author_id != crate::routes::comments::ANONYMOUS_USER_ID;
            let delete_info = mapDeleteInfo.get(&item.id).cloned();
            let author_readonly = mapAuthorReadonly
                .get(&item.author_id)
                .copied()
                .unwrap_or(true);
            CommentView {
                author_signature: mapAuthorSignatures
                    .get(&item.author_id)
                    .cloned()
                    .unwrap_or_default(),
                userpic_url: optUserpicUrl,
                userpic_width: iUserpicWidth,
                userpic_height: iUserpicHeight,
                reply: optReply,
                answer_count: iAnswerCount,
                answer_url: sAnswerUrl,
                item,
                html,
                reactions_html: reactions.html,
                show_reactions_link: reactions.show_menu_link,
                can_edit,
                can_delete,
                can_undelete,
                can_warn,
                is_topic_author,
                delete_info,
                author_readonly,
            }
        })
        .collect();

    let topic_reaction_rows: Vec<(String, i32, String, i32)> = all_reactions
        .iter()
        .filter(|(cid, ..)| cid.is_none())
        .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
        .collect();
    let topic_allow_interact = reactions_allow_interact(
        &current_user,
        reactor_frozen,
        topic_expired,
        topic.author_id,
        topic.deleted,
        false,
    );
    let topic_reactions = render_reactions_widget(
        topic.id,
        None,
        &topic_reaction_rows,
        current_user_id,
        topic_allow_interact,
        &csrf_token,
    );

    let poll = load_poll_view(
        &state,
        topic.id,
        topic.deleted,
        poll_is_pending(topic.moderate),
        topic_expired,
        query.results.unwrap_or(false),
        &current_user,
        &csrf_token,
        &topic.topic_url(),
    )
    .await?;
    let images = load_topic_images(&state, topic.id).await?;
    let sPublicUrl = state.config.public_url.trim_end_matches('/');
    let canonical_url = format!("{sPublicUrl}{}", topic.topic_url());
    let og_image_url = images.first().map_or_else(
        || format!("{sPublicUrl}/img/good-penguin.png"),
        |stImage| format!("{sPublicUrl}{}", stImage.medium_url),
    );
    let images_html = render_topic_images(
        &images,
        &topic.title,
        topic.section_prefix == "gallery",
        false,
    );
    let (comment_format_mode, comment_format_title, _) = match &current_user {
        Some(user) => user_format_mode(&state, user.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let stPostingResolution = crate::application::auth::stResolvePostingIdentity(
        &state,
        current_user.as_ref(),
        None,
        None,
    )
    .await?;
    let can_comment = stPostingResolution.stIdentity.bAuthorized
        && !comments_hidden
        && crate::routes::comments::optCommentActorError(
            &state,
            &stPostingResolution.stIdentity.stUser,
            false,
            &sRemoteIp,
        )
        .await?
        .is_none()
        && crate::routes::comments::check_comment_posting_allowed(
            &state,
            &stPostingResolution.stIdentity.stUser,
            false,
            topic.id,
        )
        .await
        .is_ok();
    let anonymous_comment_form = current_user.is_none();
    let require_comment_captcha = anonymous_comment_form
        || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let realtime_bootstrap_html = sRealtimeTopicBootstrap(
        !topic_expired && !bHasNextPage,
        topic.id,
        &topic.topic_url(),
        iRealtimeLastCommentId,
        &state.config.ws_url,
    );
    let stSimilarRemaining =
        std::time::Duration::from_millis(500).saturating_sub(stSimilarStarted.elapsed());
    let related_topics = match tokio::time::timeout(stSimilarRemaining, stSimilarTask).await {
        Ok(Ok(Ok(vecTopics))) => vecTopics,
        Ok(Ok(Err(stError))) => {
            tracing::warn!(error = %stError, topic_id = topic.id, "unable to find similar topics");
            Vec::new()
        }
        Ok(Err(stError)) => {
            tracing::warn!(error = %stError, topic_id = topic.id, "similar topics task failed");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!(
                topic_id = topic.id,
                "similar topics lookup exceeded the page deadline"
            );
            Vec::new()
        }
    };
    let sTopicUserpicHtml = mapAuthorUserpics
        .get(&topic.author_id)
        .map_or_else(String::new, |(sUrl, iWidth, iHeight)| {
            format!(
                "<div class=\"userpic\"><img class=\"photo\" src=\"{}\" alt=\"\" width=\"{iWidth}\" height=\"{iHeight}\"></div>",
                html_escape::encode_double_quoted_attribute(sUrl)
            )
        });
    let topic_card_html = sBuildTopicCardHtml(
        &state,
        &current_user,
        &csrf_token,
        StTopicCardBuildInput {
            topic: topic.clone(),
            title_plain: topic.sTitlePlain(),
            topic_author_signature,
            topic_html,
            poll,
            images_html,
            topic_reactions,
            userpic_html: sTopicUserpicHtml,
            can_comment,
            actor_frozen: reactor_frozen,
            show_menu: true,
            enable_schema: true,
            include_canonical_extras: true,
            remote_ip: sRemoteIp,
        },
    )
    .await?;

    Ok(Html(
        TopicTemplate {
            topic,
            canonical_url,
            og_image_url,
            topic_card_html,
            comments,
            pages,
            thread_root,
            show_deleted: want_deleted,
            show_deleted_button: can_view_deleted_comments && !want_deleted,
            filtered_count,
            unfiltered_count,
            csrf_token,
            comment_format_mode,
            comment_format_title,
            can_comment,
            anonymous_comment_form,
            require_comment_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            realtime_bootstrap_html,
            related_topics,
        }
        .render()?,
    )
    .into_response())
}

/// Filters `comments` down to the subtree rooted at `root` (root itself
/// plus all descendants reachable through `replyto`), sorted by id -
/// matches CommentReadService.getCommentsSubtree.
fn comment_subtree(comments: &[CommentItem], root: i32) -> Vec<CommentItem> {
    let mut ids = std::collections::HashSet::new();
    ids.insert(root);
    // `comments` is ordered by postdate ASC, so a parent always appears
    // before its replies; a couple of passes is enough to reach a fixed
    // point without needing a real graph structure.
    loop {
        let before = ids.len();
        for c in comments {
            if let Some(parent) = c.replyto
                && ids.contains(&parent)
            {
                ids.insert(c.id);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    let mut subtree: Vec<CommentItem> = comments
        .iter()
        .filter(|c| ids.contains(&c.id))
        .cloned()
        .collect();
    subtree.sort_by_key(|c| c.id);
    subtree
}

/// TopicController's inline `jumpMessage(msgid, cid, skipDeleted)`: resolves
/// which page a comment lives on (among non-deleted comments) and redirects
/// there with a `#comment-N` anchor; falls back to the deleted-comments view
/// for a moderator if the comment isn't found live.
pub(crate) async fn resolve_comment_jump(
    state: &AppState,
    topic: &TopicDetail,
    cid: i32,
    is_moderator: bool,
    current_user: &Option<UserSummary>,
) -> Result<Response> {
    let live_comments: Vec<CommentItem> = topic_service(state)
        .vecListComments(topic.id)
        .await?
        .into_iter()
        .filter(|c| !c.deleted)
        .collect();
    if let Some(pos) = live_comments.iter().position(|c| c.id == cid) {
        let per_page = messages_per_page(state, current_user).await.max(1);
        let page = pos as i64 / per_page;
        let url = if page > 0 {
            format!("{}/page{page}#comment-{cid}", topic.topic_url())
        } else {
            format!("{}#comment-{cid}", topic.topic_url())
        };
        return Ok((StatusCode::FOUND, [(header::LOCATION, url)]).into_response());
    }
    if is_moderator {
        let exists_deleted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM comments WHERE id=$1 AND topic=$2 AND deleted)",
        )
        .bind(cid)
        .bind(topic.id)
        .fetch_one(&state.pool)
        .await?;
        if exists_deleted {
            return Ok((
                StatusCode::FOUND,
                [(
                    header::LOCATION,
                    format!("{}?deleted=true#comment-{cid}", topic.topic_url()),
                )],
            )
                .into_response());
        }
    }
    Err(AppError::NotFound)
}

/// Poll.MaxPollSize
const POLL_MAX_VARIANTS: usize = 15;
/// EditTopicRequest.newPoll's default array size.
const POLL_NEW_VARIANT_SLOTS: usize = 3;

#[derive(Default, Deserialize)]
pub struct NewTopicQuery {
    pub group: Option<i32>,
    pub section: Option<i32>,
    pub tags: Option<String>,
    pub tag: Option<String>,
    pub noinfo: Option<String>,
}

pub async fn choose_topic_section(
    State(state): State<AppState>,
    Query(q): Query<NewTopicQuery>,
    CurrentUser(user): CurrentUser,
) -> Result<Response> {
    let tag = q.tags.or(q.tag).unwrap_or_default();
    if let Some(section_id) = q.section {
        let section_title: String = sqlx::query_scalar("SELECT name FROM sections WHERE id=$1")
            .bind(section_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
        type TyAddSectionRow = (i32, String, String, Option<String>, i32, i32, String);
        let rows: Vec<TyAddSectionRow> = sqlx::query_as(
            r#"SELECT g.id,g.title,g.urlname,g.info,COALESCE(g.restrict_topics,-9999),COALESCE(s.restrict_topics,-9999),
                      CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END
               FROM groups g JOIN sections s ON s.id=g.section WHERE s.id=$1 ORDER BY g.title"#,
        ).bind(section_id).fetch_all(&state.pool).await?;
        let mut choices = Vec::with_capacity(rows.len());
        for (id, title, urlname, info, group_restriction, section_restriction, section_prefix) in
            rows
        {
            let reason =
                posting_reason_for_port(&state, group_restriction.max(section_restriction), &user)
                    .await?;
            let suffix = if tag.is_empty() {
                String::new()
            } else {
                format!("&tags={}", urlencoding::encode(&tag))
            };
            choices.push(AddSectionChoice {
                title,
                url: format!("/add.jsp?group={id}{suffix}"),
                view_url: Some(format!("/{section_prefix}/{urlname}/")),
                info,
                postable: reason.is_none(),
                reason: reason.unwrap_or_default(),
            });
        }
        if choices.len() == 1 && choices[0].postable {
            return Ok(Redirect::to(&choices[0].url).into_response());
        }
        return Ok(Html(
            AddSectionTemplate {
                title: format!("{section_title}: добавление"),
                heading: format!("Добавить в «{section_title}»"),
                choices,
                choosing_groups: true,
            }
            .render()?,
        )
        .into_response());
    }

    let rows: Vec<(i32, String, i32)> =
        sqlx::query_as("SELECT id,name,COALESCE(restrict_topics,-9999) FROM sections ORDER BY id")
            .fetch_all(&state.pool)
            .await?;
    let mut choices = Vec::with_capacity(rows.len());
    for (id, title, restriction) in rows {
        let reason = posting_reason_for_port(&state, restriction, &user).await?;
        let suffix = if tag.is_empty() {
            String::new()
        } else {
            format!("&tag={}", urlencoding::encode(&tag))
        };
        choices.push(AddSectionChoice {
            title,
            url: format!("/add-section.jsp?section={id}{suffix}"),
            view_url: None,
            info: None,
            postable: reason.is_none(),
            reason: reason.unwrap_or_default(),
        });
    }
    Ok(Html(
        AddSectionTemplate {
            title: "Добавить топик".into(),
            heading: "Выберите раздел".into(),
            choices,
            choosing_groups: false,
        }
        .render()?,
    )
    .into_response())
}

struct TopicFormGroup {
    title: String,
    section_id: i32,
    links_allowed: bool,
    poll_allowed: bool,
    image_required: bool,
    image_allowed_by_section: bool,
    section_prefix: String,
    premoderated: bool,
    comments_restriction: i32,
}

type TyTopicFormGroupRow = (String, i32, bool, bool, bool, bool, String, bool, i32);

async fn load_topic_form_group(state: &AppState, group_id: i32) -> Result<TopicFormGroup> {
    let row: Option<TyTopicFormGroupRow> = sqlx::query_as(
        r#"SELECT g.title, s.id, s.havelink, COALESCE(s.vote,false), s.imagepost,
                  s.imageallowed,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  s.moderate, GREATEST(COALESCE(g.restrict_comments,-9999),
                    CASE WHEN s.id IN (1,2) THEN -9999 ELSE 45 END)
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    ).bind(group_id).fetch_optional(&state.pool).await?;
    let Some((
        title,
        section_id,
        links_allowed,
        poll_allowed,
        image_required,
        image_allowed_by_section,
        section_prefix,
        premoderated,
        comments_restriction,
    )) = row
    else {
        return Err(AppError::NotFound);
    };
    Ok(TopicFormGroup {
        title,
        section_id,
        links_allowed,
        poll_allowed,
        image_required,
        image_allowed_by_section,
        section_prefix,
        premoderated,
        comments_restriction,
    })
}

fn image_upload_allowed(group: &TopicFormGroup, user: &Option<UserSummary>) -> bool {
    group.image_required
        || (group.image_allowed_by_section
            && user
                .as_ref()
                .is_some_and(|u| u.canmod || u.corrector || u.score.unwrap_or(0) >= 50))
}

fn sTopicCount(iCount: i32) -> String {
    let sNoun = if (10..=20).contains(&(iCount.rem_euclid(100))) {
        "топиков"
    } else {
        match iCount.rem_euclid(10) {
            1 => "топик",
            2..=4 => "топика",
            _ => "топиков",
        }
    };
    format!("{iCount}\u{a0}{sNoun}")
}

fn topicLimitNotices(stInfo: StTopicLimitInfo) -> (Option<String>, Option<String>) {
    if stInfo.bExempt {
        (None, None)
    } else if stInfo.bReached {
        (
            Some(format!(
                "Вы можете разместить не более {} за 24 часа. Сейчас вы можете подготовить текст и сохранить его черновик",
                sTopicCount(stInfo.iLimit)
            )),
            None,
        )
    } else if stInfo.iCurrentCount > 0 {
        (
            None,
            Some(format!(
                "Вы разместили {} из {} за 24 часа",
                sTopicCount(stInfo.iCurrentCount),
                stInfo.iLimit
            )),
        )
    } else {
        (None, None)
    }
}

pub async fn new_topic_form(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<NewTopicQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let selected_group = match q.group {
        Some(id) => id,
        None => return Ok(Redirect::to("/add-section.jsp").into_response()),
    };
    let stResolution =
        crate::application::auth::stResolvePostingIdentity(&state, user.as_ref(), None, None)
            .await?;
    let stPostingActor = stPostingActor(&stResolution.stIdentity);
    let stPostingPermission = add_topic_service(&state)
        .optCheckGroup(selected_group, stPostingActor, &sRemoteIp)
        .await?
        .ok_or(AppError::NotFound)?;
    let group = load_topic_form_group(&state, selected_group).await?;
    let stTopicLimitInfo = state
        .topic_publish
        .stTopicLimitInfo(stPostingActor, group.section_id)
        .await?;
    let stPublishPermission = state
        .topic_publish
        .stCheckPublish(stPostingPermission, stTopicLimitInfo);
    let (topic_limit_error, topic_limit_info) = topicLimitNotices(stTopicLimitInfo);
    let (format_mode, format_mode_title, _) = match &user {
        Some(user) => user_format_mode(&state, user.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let image_allowed = image_upload_allowed(&group, &user);
    let noinfo = q.noinfo.is_some();
    let initial_tags = q.tags.or(q.tag).unwrap_or_default();
    let add_info_html = if noinfo {
        None
    } else {
        let path = format!(
            "{}/help/new-topic-{}.md",
            state.config.static_dir, group.section_prefix
        );
        tokio::fs::read_to_string(path)
            .await
            .ok()
            .map(|source| markup::render_markdown(&source))
    };
    Ok(Html(
        TopicFormTemplate {
            title: format!("Добавить в «{}»", group.title),
            form_error: None,
            topic_limit_error,
            topic_limit_info,
            topic_posting_allowed: stPublishPermission.bPermitted(),
            topic_posting_reason: stPublishPermission.sReason().to_string(),
            action: "/add.jsp".into(),
            topic_id: None,
            csrf_token,
            poll_variants: Vec::new(),
            poll_new_rows: if group.poll_allowed {
                vec![String::new(); POLL_MAX_VARIANTS]
            } else {
                Vec::new()
            },
            poll_multiselect: false,
            selected_group,
            is_edit: false,
            links_allowed: group.links_allowed,
            premoderated: group.premoderated,
            poll_allowed: group.poll_allowed,
            image_allowed,
            image_required: group.image_required,
            additional_image_rows: if image_allowed && group.section_prefix != "forum" {
                vec![(); 3]
            } else {
                Vec::new()
            },
            existing_images: Vec::new(),
            uploaded_images: Vec::new(),
            form_title: String::new(),
            form_msg: String::new(),
            form_url: String::new(),
            form_linktext: String::new(),
            form_tags: initial_tags.clone(),
            preview_html: None,
            noinfo,
            add_info_html,
            format_mode,
            format_mode_title,
            anonymous_form: user.is_none(),
            form_nick: "anonymous".into(),
            require_captcha: user.is_none()
                || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            show_allow_anonymous: user.is_some()
                && !group.premoderated
                && group.comments_restriction < -50,
            allow_anonymous: true,
        }
        .render()?,
    )
    .into_response())
}

/// AddTopicController.MaxMessageLength / MaxMessageLengthAnonymous.
const TOPIC_MAX_MESSAGE_LENGTH: usize = 65536;
const TOPIC_MAX_MESSAGE_LENGTH_ANONYMOUS: usize = 8196;

struct TopicUpload {
    bytes: bytes::Bytes,
}

async fn parse_topic_request(
    state: &AppState,
    request: Request,
) -> Result<(Vec<(String, String)>, Vec<TopicUpload>)> {
    let multipart_request = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if !multipart_request {
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .map_err(|error| AppError::BadRequest(format!("invalid body: {error}")))?;
        return Ok((crate::form::parse_pairs(&bytes)?, Vec::new()));
    }

    let mut multipart = Multipart::from_request(request, state)
        .await
        .map_err(|error| AppError::BadRequest(format!("ошибка multipart: {error}")))?;
    let mut pairs = Vec::new();
    let mut uploads = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::BadRequest(format!("ошибка multipart: {error}")))?
    {
        let Some(name) = field.name().map(str::to_string) else {
            continue;
        };
        if name == "image" || name == "additionalImage" || name == "images" {
            let bytes = field.bytes().await.map_err(|error| {
                AppError::BadRequest(format!("ошибка чтения изображения: {error}"))
            })?;
            if !bytes.is_empty() {
                uploads.push(TopicUpload { bytes });
            }
        } else {
            let value = field.text().await.map_err(|error| {
                AppError::BadRequest(format!("ошибка чтения поля {name}: {error}"))
            })?;
            pairs.push((name, value));
        }
    }
    Ok((pairs, uploads))
}

fn validate_topic_form(form: &TopicForm, links_allowed: bool, bAnonymous: bool) -> Result<()> {
    let title = form.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest(
            "заголовок сообщения не может быть пустым".into(),
        ));
    }
    if crate::domain::title::iJavaStringLength(&form.title) > 140 {
        return Err(AppError::BadRequest("Слишком большой заголовок".into()));
    }
    if title.starts_with('[') {
        return Err(AppError::BadRequest(
            "Не добавляйте теги в заголовки, используйте предназначенное для тегов поле ввода"
                .into(),
        ));
    }
    let iMaxMessageLength = if bAnonymous {
        TOPIC_MAX_MESSAGE_LENGTH_ANONYMOUS
    } else {
        TOPIC_MAX_MESSAGE_LENGTH
    };
    if form.msg.chars().count() > iMaxMessageLength {
        return Err(AppError::BadRequest("Слишком большое сообщение".into()));
    }
    if links_allowed && let Some(url) = form.url.as_deref().filter(|value| !value.trim().is_empty())
    {
        if url.chars().count() > 255 {
            return Err(AppError::BadRequest("Слишком длинный URL".into()));
        }
        if reqwest::Url::parse(url).is_err() {
            return Err(AppError::BadRequest("Некорректный URL".into()));
        }
        if form.linktext.as_deref().unwrap_or("").is_empty() {
            return Err(AppError::BadRequest("URL указан без текста ссылки".into()));
        }
    }
    Ok(())
}

fn validate_topic_image(data: &[u8]) -> Result<(image::DynamicImage, &'static str)> {
    use image::GenericImageView;
    const MAX_FILE_SIZE: usize = 8 * 1024 * 1024;
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: слишком большой файл".into(),
        ));
    }
    let format = image::guess_format(data)
        .map_err(|_| AppError::BadRequest("Некорректное изображение: неизвестный формат".into()))?;
    let extension = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::Gif => "gif",
        _ => {
            return Err(AppError::BadRequest(
                "Некорректное изображение: поддерживаются jpeg, gif и png".into(),
            ));
        }
    };
    let image =
        crate::image_upload::stDecodeWithLimits(data, format, 5120, 5120, 256 * 1024 * 1024)
            .map_err(|error| AppError::BadRequest(format!("Некорректное изображение: {error}")))?;
    let (width, height) = image.dimensions();
    if !(400..=5120).contains(&width) || !(400..=5120).contains(&height) {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: недопустимые размеры изображения".into(),
        ));
    }
    if f64::from(height) / (f64::from(width) + 1.0) > 2.3 {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: слишком узкое изображение".into(),
        ));
    }
    if f64::from(width) / (f64::from(height) + 1.0) > 5.0 {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: слишком широкое изображение".into(),
        ));
    }
    Ok((image, extension))
}

/// `ImageUtil.resizeImage(..., Scalr.Mode.FIT_TO_WIDTH, size)` creates every
/// derivative at the requested width (portrait images may therefore be taller
/// than `size`) and paints transparent pixels on white before JPEG encoding.
fn vecEncodeTopicDerivative(stImage: &image::DynamicImage, iWidth: u32) -> Result<Vec<u8>> {
    use image::GenericImageView;

    let (iSourceWidth, iSourceHeight) = stImage.dimensions();
    let iHeight = ((u64::from(iSourceHeight) * u64::from(iWidth) + u64::from(iSourceWidth) / 2)
        / u64::from(iSourceWidth))
    .max(1) as u32;
    let stRgba = image::imageops::resize(
        &stImage.to_rgba8(),
        iWidth,
        iHeight,
        image::imageops::FilterType::Lanczos3,
    );
    let mut stRgb = image::RgbImage::new(iWidth, iHeight);
    for (iX, iY, stPixel) in stRgba.enumerate_pixels() {
        let iAlpha = u16::from(stPixel[3]);
        let mut arrRgb = [0u8; 3];
        for iChannel in 0..3 {
            arrRgb[iChannel] =
                ((u16::from(stPixel[iChannel]) * iAlpha + 255 * (255 - iAlpha) + 127) / 255) as u8;
        }
        stRgb.put_pixel(iX, iY, image::Rgb(arrRgb));
    }

    let mut vecEncoded = Vec::new();
    image::DynamicImage::ImageRgb8(stRgb)
        .write_to(
            &mut std::io::Cursor::new(&mut vecEncoded),
            image::ImageFormat::Jpeg,
        )
        .map_err(|stError| AppError::Anyhow(stError.into()))?;
    Ok(vecEncoded)
}

async fn save_topic_upload(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    topic_id: i32,
    upload: &TopicUpload,
    stRollback: &mut StTopicUploadRollback,
) -> Result<()> {
    let (image, extension) = validate_topic_image(&upload.bytes)?;
    let image_id: i32 =
        sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('images','id'))::int")
            .fetch_one(&mut **tx)
            .await?;
    let relative_dir = format!("images/{image_id}");
    let directory = format!("{}/{relative_dir}", state.config.upload_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Anyhow(error.into()))?;
    stRollback.vTrack(directory.clone());
    tokio::fs::write(format!("{directory}/original.{extension}"), &upload.bytes)
        .await
        .map_err(|error| AppError::Anyhow(error.into()))?;
    for size in [500u32, 1000, 1500, 2000] {
        let encoded = vecEncodeTopicDerivative(&image, size)?;
        tokio::fs::write(format!("{directory}/{size}px.jpg"), encoded)
            .await
            .map_err(|error| AppError::Anyhow(error.into()))?;
    }
    sqlx::query("INSERT INTO images(id,topic,extension,main) VALUES($1,$2,$3,false)")
        .bind(image_id)
        .bind(topic_id)
        .bind(extension)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn vecTopicPreviewPaths(stState: &AppState, sName: &str) -> Vec<std::path::PathBuf> {
    let stDirectory = std::path::Path::new(&stState.config.upload_dir).join("gallery/preview");
    let sStem = sName.rsplit_once('.').map_or(sName, |(sStem, _)| sStem);
    let mut vecPaths = vec![stDirectory.join(sName)];
    vecPaths.extend(
        [500u32, 1000, 1500, 2000].map(|iSize| stDirectory.join(format!("{sStem}-{iSize}px.jpg"))),
    );
    vecPaths
}

fn vecReusableTopicPreviews(stState: &AppState, iUserId: i32, vecNames: &[String]) -> Vec<String> {
    let stPattern = regex::Regex::new(&format!(
        r"^preview-{}-[\w-]+\.(?:jpg|png|gif)$",
        regex::escape(&iUserId.to_string())
    ))
    .expect("static topic preview pattern");
    vecNames
        .iter()
        .filter(|sName| stPattern.is_match(sName))
        // ImageService.processUpload reuses a preview when its main file is
        // present; saveImage will then validate/move every derivative.
        .filter(|sName| vecTopicPreviewPaths(stState, sName)[0].is_file())
        .cloned()
        .collect()
}

async fn vecStageTopicPreviews(
    stState: &AppState,
    iUserId: i32,
    vecUploads: &[TopicUpload],
) -> Result<Vec<String>> {
    let stDirectory = std::path::Path::new(&stState.config.upload_dir).join("gallery/preview");
    tokio::fs::create_dir_all(&stDirectory)
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?;
    let mut vecCreatedNames = Vec::new();
    for stUpload in vecUploads {
        let (stImage, sExtension) = validate_topic_image(&stUpload.bytes)?;
        let sName = format!(
            "preview-{iUserId}-{}.{}",
            uuid::Uuid::new_v4().simple(),
            sExtension
        );
        let vecPaths = vecTopicPreviewPaths(stState, &sName);
        let stResult: Result<String> = async {
            tokio::fs::write(&vecPaths[0], &stUpload.bytes)
                .await
                .map_err(|stError| AppError::Anyhow(stError.into()))?;
            for (stPath, iSize) in vecPaths.iter().skip(1).zip([500u32, 1000, 1500, 2000]) {
                tokio::fs::write(stPath, vecEncodeTopicDerivative(&stImage, iSize)?)
                    .await
                    .map_err(|stError| AppError::Anyhow(stError.into()))?;
            }
            Ok(sName.clone())
        }
        .await;
        match stResult {
            Ok(sName) => vecCreatedNames.push(sName),
            Err(stError) => {
                vDeleteTopicPreview(stState, &sName).await;
                for sName in &vecCreatedNames {
                    vDeleteTopicPreview(stState, sName).await;
                }
                return Err(stError);
            }
        }
    }
    Ok(vecCreatedNames)
}

async fn save_topic_preview(
    stTransaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    stState: &AppState,
    iTopicId: i32,
    sName: &str,
    stRollback: &mut StTopicUploadRollback,
) -> Result<()> {
    let sExtension = sName
        .rsplit_once('.')
        .map(|(_, sExtension)| sExtension)
        .ok_or_else(|| AppError::BadRequest("Некорректное имя preview изображения".into()))?;
    let vecSourcePaths = vecTopicPreviewPaths(stState, sName);
    if vecSourcePaths.iter().any(|stPath| !stPath.is_file()) {
        return Err(AppError::BadRequest(
            "Preview изображения истёк или повреждён".into(),
        ));
    }
    let iImageId: i32 =
        sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('images','id'))::int")
            .fetch_one(&mut **stTransaction)
            .await?;
    let stDirectory = std::path::Path::new(&stState.config.upload_dir)
        .join("images")
        .join(iImageId.to_string());
    tokio::fs::create_dir(&stDirectory)
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?;
    stRollback.vTrack(stDirectory.to_string_lossy().into_owned());
    tokio::fs::copy(
        &vecSourcePaths[0],
        stDirectory.join(format!("original.{sExtension}")),
    )
    .await
    .map_err(|stError| AppError::Anyhow(stError.into()))?;
    for (stSource, iSize) in vecSourcePaths
        .iter()
        .skip(1)
        .zip([500u32, 1000, 1500, 2000])
    {
        tokio::fs::copy(stSource, stDirectory.join(format!("{iSize}px.jpg")))
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?;
    }
    sqlx::query("INSERT INTO images(id,topic,extension,main) VALUES($1,$2,$3,false)")
        .bind(iImageId)
        .bind(iTopicId)
        .bind(sExtension)
        .execute(&mut **stTransaction)
        .await?;
    Ok(())
}

async fn vDeleteTopicPreview(stState: &AppState, sName: &str) {
    for stPath in vecTopicPreviewPaths(stState, sName) {
        if let Err(stError) = tokio::fs::remove_file(&stPath).await
            && stError.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %stPath.display(), error = %stError, "failed to remove consumed topic preview");
        }
    }
}

/// Filesystem writes cannot participate in PostgreSQL transactions. Track
/// every newly-created image directory and remove it if any later SQL, image
/// or commit step fails, so a rejected topic/edit does not leak orphan media.
struct StTopicUploadRollback {
    vecDirectories: Vec<String>,
    bCommitted: bool,
}

impl StTopicUploadRollback {
    fn stNew() -> Self {
        Self {
            vecDirectories: Vec::new(),
            bCommitted: false,
        }
    }

    fn vTrack(&mut self, sDirectory: String) {
        self.vecDirectories.push(sDirectory);
    }

    fn vCommit(&mut self) {
        self.bCommitted = true;
    }
}

impl Drop for StTopicUploadRollback {
    fn drop(&mut self) {
        if self.bCommitted {
            return;
        }
        for sDirectory in self.vecDirectories.iter().rev() {
            if let Err(stError) = std::fs::remove_dir_all(sDirectory)
                && stError.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!(path = %sDirectory, error = %stError, "failed to remove rolled-back topic image directory");
            }
        }
    }
}

#[cfg(test)]
mod topic_image_processing_tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn derivatives_fit_width_like_java_for_portrait_images() {
        let stImage = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            400,
            800,
            image::Rgba([10, 20, 30, 255]),
        ));
        let vecJpeg = vecEncodeTopicDerivative(&stImage, 500).expect("encode derivative");
        assert_eq!(
            image::load_from_memory(&vecJpeg).unwrap().dimensions(),
            (500, 1000)
        );
    }

    #[test]
    fn transparent_derivative_pixels_are_composited_on_white() {
        let stImage = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            400,
            400,
            image::Rgba([0, 0, 0, 0]),
        ));
        let vecJpeg = vecEncodeTopicDerivative(&stImage, 500).expect("encode derivative");
        let stDecoded = image::load_from_memory(&vecJpeg).unwrap().to_rgb8();
        let stPixel = stDecoded.get_pixel(250, 250);
        assert!(stPixel.0.into_iter().all(|iChannel| iChannel >= 250));
    }

    #[test]
    fn failed_database_flow_removes_staged_image_directory() {
        let pathDirectory =
            std::env::temp_dir().join(format!("lorsource-topic-upload-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&pathDirectory).unwrap();
        std::fs::write(pathDirectory.join("original.png"), b"test").unwrap();
        {
            let mut stRollback = StTopicUploadRollback::stNew();
            stRollback.vTrack(pathDirectory.to_string_lossy().into_owned());
        }
        assert!(!pathDirectory.exists());
    }
}

async fn renderSubmittedAddTopicForm(
    stState: &AppState,
    stGroup: &TopicFormGroup,
    stForm: &TopicForm,
    sCsrfToken: &str,
    sFormatMode: &str,
    sFormatModeTitle: &str,
    sMarkupId: &str,
    bUploadAllowed: bool,
    optFormError: Option<String>,
    stTopicLimitInfo: StTopicLimitInfo,
    stPublishPermission: &StAddTopicPermission,
    bPreviewNofollow: bool,
    bPreview: bool,
    bSessionAuthorized: bool,
    bRequireCaptcha: bool,
) -> Result<Response> {
    let (optTopicLimitError, optTopicLimitInfo) = topicLimitNotices(stTopicLimitInfo);
    let optPreviewHtml = if bPreview {
        let stMarkupUsers = stState
            .markup
            .stResolveBatch([(&*stForm.msg, sMarkupId)])
            .await?;
        Some(markup::render_message_with_markup_policy_and_users(
            &stForm.msg,
            Some(sMarkupId),
            None,
            bPreviewNofollow,
            Some(&stState.config.public_url),
            Some(&stMarkupUsers),
        ))
    } else {
        None
    };
    Ok(Html(
        TopicFormTemplate {
            title: format!("Добавить в «{}»", stGroup.title),
            form_error: optFormError,
            topic_limit_error: optTopicLimitError,
            topic_limit_info: optTopicLimitInfo,
            topic_posting_allowed: stPublishPermission.bPermitted(),
            topic_posting_reason: stPublishPermission.sReason().to_string(),
            action: "/add.jsp".into(),
            topic_id: None,
            csrf_token: sCsrfToken.to_string(),
            poll_variants: Vec::new(),
            poll_new_rows: if stGroup.poll_allowed {
                stForm.poll.clone()
            } else {
                Vec::new()
            },
            poll_multiselect: stForm.multiselect.is_some(),
            selected_group: stForm.group,
            is_edit: false,
            links_allowed: stGroup.links_allowed,
            premoderated: stGroup.premoderated,
            poll_allowed: stGroup.poll_allowed,
            image_allowed: bUploadAllowed,
            image_required: stGroup.image_required,
            additional_image_rows: if bUploadAllowed && stGroup.section_prefix != "forum" {
                vec![(); 3]
            } else {
                Vec::new()
            },
            existing_images: Vec::new(),
            uploaded_images: stForm.uploaded_images.clone(),
            form_title: stForm.title.clone(),
            form_msg: stForm.msg.clone(),
            form_url: stForm.url.clone().unwrap_or_default(),
            form_linktext: stForm.linktext.clone().unwrap_or_default(),
            form_tags: stForm.tags.clone().unwrap_or_default(),
            preview_html: optPreviewHtml,
            noinfo: stForm
                .noinfo
                .as_deref()
                .is_some_and(|sValue| matches!(sValue, "1" | "true" | "on")),
            add_info_html: None,
            format_mode: sFormatMode.to_string(),
            format_mode_title: sFormatModeTitle.to_string(),
            anonymous_form: !bSessionAuthorized,
            form_nick: stForm.nick.clone().unwrap_or_else(|| "anonymous".into()),
            require_captcha: bRequireCaptcha,
            captcha_site_key: stState
                .config
                .captcha_public_key
                .clone()
                .unwrap_or_default(),
            show_allow_anonymous: bSessionAuthorized
                && !stGroup.premoderated
                && stGroup.comments_restriction < -50,
            allow_anonymous: stForm.allow_anonymous.is_some(),
        }
        .render()?,
    )
    .into_response())
}

pub async fn create_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    request: Request,
) -> Result<Response> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        request.headers(),
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let sUserAgent = request
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|stValue| stValue.to_str().ok())
        .map(str::to_owned);
    let (pairs, uploads) = parse_topic_request(&state, request).await?;
    let mut form = parse_topic_form(&pairs)?;
    let group = load_topic_form_group(&state, form.group).await?;
    let bSessionAuthorized = user.is_some();
    let bShowAllowAnonymous =
        bSessionAuthorized && !group.premoderated && group.comments_restriction < -50;
    let bAllowAnonymous = !bShowAllowAnonymous || form.allow_anonymous.is_some();
    let bRequireCaptcha =
        !bSessionAuthorized || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let mut optFormError = None;
    if form.preview.is_none()
        && bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
    {
        optFormError = Some(sError);
    }
    let stResolution = crate::application::auth::stResolvePostingIdentity(
        &state,
        user.as_ref(),
        form.nick.as_deref(),
        form.password.as_deref(),
    )
    .await?;
    if optFormError.is_none() {
        optFormError = stResolution.optError.clone();
    }
    let stPostingIdentity = stResolution.stIdentity;
    let bPostingAuthorFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until > CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(stPostingIdentity.stUser.id)
    .fetch_one(&state.pool)
    .await?;
    let bPreviewNofollow = !crate::domain::topic::link_policy::StAuthorLinkState {
        iScore: stPostingIdentity.stUser.score.unwrap_or(0),
        bBlocked: stPostingIdentity.stUser.blocked.unwrap_or(false),
        bAnonymous: !stPostingIdentity.bAuthorized,
        bFrozen: bPostingAuthorFrozen,
    }
    .bFollowInTopic(false);
    // AuthUtil.postingUser deliberately does not change the site profile:
    // credentialed public-form posts use Profile.DEFAULT, while a real HTTP
    // session retains its selected markup mode.
    let (format_mode, format_mode_title, markup_id) = match user.as_ref() {
        Some(stUser) => user_format_mode(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let stPostingActor = stPostingActor(&stPostingIdentity);
    let stPostingPermission = add_topic_service(&state)
        .optCheckGroup(form.group, stPostingActor, &sRemoteIp)
        .await?
        .ok_or(AppError::NotFound)?;
    let stTopicLimitInfo = state
        .topic_publish
        .stTopicLimitInfo(stPostingActor, group.section_id)
        .await?;
    let stPublishPermission = state
        .topic_publish
        .stCheckPublish(stPostingPermission.clone(), stTopicLimitInfo);
    let optUploadUser = stPostingIdentity
        .bAuthorized
        .then(|| stPostingIdentity.stUser.clone());
    let upload_allowed = image_upload_allowed(&group, &optUploadUser);
    if optFormError.is_some() {
        return renderSubmittedAddTopicForm(
            &state,
            &group,
            &form,
            &csrf_token,
            &format_mode,
            &format_mode_title,
            &markup_id,
            upload_allowed,
            optFormError,
            stTopicLimitInfo,
            &stPublishPermission,
            bPreviewNofollow,
            form.preview.is_some(),
            bSessionAuthorized,
            bRequireCaptcha,
        )
        .await;
    }
    if !stPostingPermission.bPermitted() {
        // AddTopicController.checkOrError puts the restriction into the
        // BindingResult and returns the populated form (HTTP 200), including
        // in preview mode.  It does not attempt the mutation.
        return renderSubmittedAddTopicForm(
            &state,
            &group,
            &form,
            &csrf_token,
            &format_mode,
            &format_mode_title,
            &markup_id,
            upload_allowed,
            Some(format!(
                "Недостаточно прав для создания топика: {}",
                stPostingPermission.sReason()
            )),
            stTopicLimitInfo,
            &stPublishPermission,
            bPreviewNofollow,
            form.preview.is_some(),
            bSessionAuthorized,
            bRequireCaptcha,
        )
        .await;
    }
    if form.preview.is_none()
        && crate::form::get(&pairs, "csrf").map(str::trim) != Some(csrf_token.trim())
    {
        return Err(AppError::Forbidden);
    }
    validate_topic_form(&form, group.links_allowed, !stPostingIdentity.bAuthorized)?;
    let is_draft = form.draft.is_some();
    let premoderated: bool = sqlx::query_scalar(
        "SELECT s.moderate FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1",
    )
    .bind(form.group)
    .fetch_one(&state.pool)
    .await?;
    if (!uploads.is_empty() || !form.uploaded_images.is_empty()) && !upload_allowed {
        return Err(AppError::Forbidden);
    }
    form.uploaded_images =
        vecReusableTopicPreviews(&state, stPostingIdentity.stUser.id, &form.uploaded_images);
    if form.uploaded_images.len() + uploads.len() > 4 {
        return Err(AppError::BadRequest("Слишком много изображений".into()));
    }
    if group.image_required && form.uploaded_images.is_empty() && uploads.is_empty() {
        return Err(AppError::BadRequest("Изображение отсутствует".into()));
    }

    if form.preview.is_some() {
        form.uploaded_images
            .extend(vecStageTopicPreviews(&state, stPostingIdentity.stUser.id, &uploads).await?);
        return renderSubmittedAddTopicForm(
            &state,
            &group,
            &form,
            &csrf_token,
            &format_mode,
            &format_mode_title,
            &markup_id,
            upload_allowed,
            None,
            stTopicLimitInfo,
            &stPublishPermission,
            bPreviewNofollow,
            true,
            bSessionAuthorized,
            bRequireCaptcha,
        )
        .await;
    }

    // AddTopicRequestValidator.validateTags/AddTopicController: every
    // topic needs 1-5 valid tags, and creating a brand-new tag (one that
    // doesn't already exist as a value or synonym) needs either a
    // premoderated section or score>=200 (GroupPermissionService.canCreateTag).
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(
        &state,
        &tags,
        &stPostingIdentity.stUser,
        premoderated,
    )
    .await?;

    // AddTopicController performs FloodProtector.AddTopic after all ordinary
    // validation and CSRF checks.  A draft is rate-limited too; only preview
    // bypasses the cache (it returned above).  A successful rate check is
    // deliberately recorded before TopicPublishChecker, so a daily-limit
    // rejection consumes the same IP interval as it does in Java.
    if let Some(sRateError) = state
        .topic_publish
        .optCheckAddTopicRate(stPostingActor, &sRemoteIp)
        .await?
    {
        return renderSubmittedAddTopicForm(
            &state,
            &group,
            &form,
            &csrf_token,
            &format_mode,
            &format_mode_title,
            &markup_id,
            upload_allowed,
            Some(sRateError),
            stTopicLimitInfo,
            &stPublishPermission,
            bPreviewNofollow,
            false,
            bSessionAuthorized,
            bRequireCaptcha,
        )
        .await;
    }

    if !is_draft && !stPublishPermission.bPermitted() {
        return renderSubmittedAddTopicForm(
            &state,
            &group,
            &form,
            &csrf_token,
            &format_mode,
            &format_mode_title,
            &markup_id,
            upload_allowed,
            Some(format!("Ограничение: {}", stPublishPermission.sReason())),
            stTopicLimitInfo,
            &stPublishPermission,
            bPreviewNofollow,
            false,
            bSessionAuthorized,
            bRequireCaptcha,
        )
        .await;
    }

    let sStoredTitle = crate::domain::title::sEscapeForStorage(&form.title);
    let mut stUploadRollback = StTopicUploadRollback::stNew();
    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    let id = service.iNextMessageId(&mut tx).await?;
    service
        .vInsertTopicMessage(&mut tx, id, &form.msg, &markup_id)
        .await?;
    service
        .vInsertTopic(
            &mut tx,
            StNewTopic {
                iMsgId: id,
                iGroupId: form.group,
                iUserId: stPostingIdentity.stUser.id,
                sTitle: &sStoredTitle,
                optUrl: group
                    .links_allowed
                    .then_some(form.url.as_deref())
                    .flatten()
                    .filter(|sValue| !sValue.trim().is_empty()),
                optLinkText: group
                    .links_allowed
                    .then_some(form.linktext.as_deref())
                    .flatten()
                    .filter(|sValue| !sValue.trim().is_empty()),
                bDraft: is_draft,
                sPostIp: &sRemoteIp,
                optUserAgent: sUserAgent.as_deref(),
                bAllowAnonymous,
            },
        )
        .await?;
    service
        .vReplaceTags(&mut tx, id, form.tags.as_deref())
        .await?;
    if group.poll_allowed {
        // AddTopicController.preparePollPreview/TopicService.addMessage:
        // every submitted variant_id is 0 (new) on creation.
        let variant_ids = vec![0; form.poll.len()];
        save_poll(
            &mut tx,
            id,
            form.multiselect.is_some(),
            &variant_ids,
            &form.poll,
        )
        .await?;
    }
    for upload in &uploads {
        save_topic_upload(&mut tx, &state, id, upload, &mut stUploadRollback).await?;
    }
    for sPreview in &form.uploaded_images {
        save_topic_preview(&mut tx, &state, id, sPreview, &mut stUploadRollback).await?;
    }
    let vecNotified = if !is_draft {
        // TopicService.addMessage keeps notification rows in the same local
        // transaction as the topic itself. A failure must not leave a topic
        // committed without its matching REF/TAG bookkeeping.
        notify_topic_users_tx(
            &mut tx,
            id,
            stPostingIdentity.stUser.id,
            &form.msg,
            &markup_id,
            !premoderated,
        )
        .await?
    } else {
        Vec::new()
    };
    tx.commit().await?;
    stUploadRollback.vCommit();
    for sPreview in &form.uploaded_images {
        vDeleteTopicPreview(&state, sPreview).await;
    }
    if !is_draft {
        state.realtime.vNotifyEvents(vecNotified.iter().copied());
    }
    crate::search_index::index_topic(&state, id, false).await;
    // Java shows a dedicated confirmation for protected sections because
    // the new topic is intentionally absent from the public section until
    // a moderator commits it.
    let topic = get_topic(&state, id).await?;
    if premoderated && !is_draft {
        return Ok(Html(
            ModeratedTopicTemplate {
                topic_url: topic.topic_url(),
            }
            .render()?,
        )
        .into_response());
    }
    Ok(Redirect::to(&topic.topic_url()).into_response())
}

#[derive(Template)]
#[template(path = "topic_created_moderated.html")]
struct ModeratedTopicTemplate {
    topic_url: String,
}

#[derive(Debug, Clone)]
struct StEditPollVariantView {
    id: i32,
    label: String,
}

#[derive(Debug, Clone)]
struct StEditGroupView {
    id: i32,
    title: String,
    selected: bool,
}

#[derive(Debug, Clone)]
struct StEditEditorView {
    nick: String,
    score: i32,
    bonus: i32,
    blocked: bool,
}

#[derive(Debug, Clone)]
struct StEditTopicFormValues {
    optTitle: Option<String>,
    optMessage: Option<String>,
    optUrl: Option<String>,
    optLinkText: Option<String>,
    optTagsRaw: Option<String>,
    vecPoll: Vec<StTopicEditPollValue>,
    vecNewPoll: Vec<String>,
    bPollMapPresent: bool,
    bMultiSelect: bool,
    bMinor: bool,
    iBonus: i32,
    vecEditorBonus: Vec<(String, i32)>,
    optChangeGroupId: Option<i32>,
    optLastEditMillis: Option<i64>,
    vecUploadedImages: Vec<String>,
}

impl StEditTopicFormValues {
    fn stInitial(stPrepared: &StPreparedTopicEdit, sMessage: String) -> Self {
        let stTopic = &stPrepared.stSnapshot;
        let (vecPoll, bMultiSelect) = stTopic.optPoll.as_ref().map_or_else(
            || (Vec::new(), false),
            |stPoll| {
                (
                    stPoll
                        .vecVariants
                        .iter()
                        .map(|stVariant| StTopicEditPollValue {
                            iVariantId: stVariant.iId,
                            sLabel: stVariant.sLabel.clone(),
                        })
                        .collect(),
                    stPoll.bMultiSelect,
                )
            },
        );
        Self {
            optTitle: Some(crate::domain::title::sUnescapeFromStorage(
                &stTopic.sStoredTitle,
            )),
            optMessage: Some(sMessage),
            optUrl: stTopic.optUrl.clone(),
            optLinkText: stTopic.optLinkText.clone(),
            optTagsRaw: (!stTopic.vecTags.is_empty()).then(|| stTopic.vecTags.join(", ")),
            vecPoll,
            vecNewPoll: vec![String::new(); POLL_NEW_VARIANT_SLOTS],
            bPollMapPresent: stTopic.optPoll.is_some(),
            bMultiSelect,
            bMinor: stTopic.bMinor,
            iBonus: 3,
            vecEditorBonus: stTopic
                .vecEditors
                .iter()
                .map(|stEditor| (stEditor.sNick.clone(), 0))
                .collect(),
            optChangeGroupId: Some(stTopic.iGroupId),
            optLastEditMillis: stTopic.optLastEditMillis,
            vecUploadedImages: Vec::new(),
        }
    }
}

#[derive(Template)]
#[template(path = "edit_topic.html")]
struct StEditTopicTemplate {
    heading: String,
    csrf_token: String,
    errors: Vec<String>,
    topic_id: i32,
    last_edit: Option<i64>,
    content_editable: bool,
    tags_editable: bool,
    mini_editable: bool,
    links_allowed: bool,
    poll_allowed: bool,
    imagepost: bool,
    existing_images: Vec<TopicImageView>,
    uploaded_images: Vec<String>,
    empty_image_slots: Vec<usize>,
    form_title: String,
    form_msg: String,
    form_url: String,
    form_linktext: String,
    form_tags: String,
    form_minor: bool,
    poll_variants: Vec<StEditPollVariantView>,
    new_poll: Vec<String>,
    poll_multiselect: bool,
    format_mode: String,
    format_title: String,
    topic_card_html: Option<String>,
    draft: bool,
    publish_allowed: bool,
    publish_reason: String,
    commit_form: bool,
    groups: Vec<StEditGroupView>,
    author_nick: String,
    author_score: i32,
    author_blocked: bool,
    bonus: i32,
    editors: Vec<StEditEditorView>,
}

#[derive(Template)]
#[template(path = "topic_edit_user_error.html")]
struct StTopicEditUserErrorTemplate {
    exception_class: &'static str,
    message: String,
}

fn stTopicEditUserErrorResponse(sExceptionClass: &'static str, sMessage: String) -> Response {
    let sBody = StTopicEditUserErrorTemplate {
        exception_class: sExceptionClass,
        message: sMessage,
    }
    .render()
    .unwrap_or_else(|_| "Внутренняя ошибка сервера".to_owned());
    (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response()
}

fn sEditPreviewTitle(sRawTitle: &str) -> String {
    // Topic.fromEditRequest escapes the raw title, then the preview's
    // <l:title> runs processTitle without the DB-read makeTitle pass.
    crate::domain::title::sProcessTitlePlainForDisplay(&crate::domain::title::sEscapeForStorage(
        sRawTitle,
    ))
}

fn stEditPreviewHeaderValues(
    optSubmittedTitle: Option<&str>,
    sStoredTitle: &str,
    optSubmittedUrl: Option<&str>,
    optStoredUrl: Option<&str>,
    optSubmittedLinkText: Option<&str>,
    optStoredLinkText: Option<&str>,
) -> (String, String, String) {
    let sTitle = optSubmittedTitle.map_or_else(
        || crate::domain::title::sTopicTitlePlainForDisplay(sStoredTitle),
        sEditPreviewTitle,
    );
    let sUrl = optSubmittedUrl
        .map(crate::application::topic::edit::sFixUrlLikeJava)
        .or_else(|| optStoredUrl.map(str::to_owned))
        .unwrap_or_default();
    let sLinkText = optSubmittedLinkText
        .map(str::to_owned)
        .or_else(|| optStoredLinkText.map(str::to_owned))
        .unwrap_or_default();
    (sTitle, sUrl, sLinkText)
}

fn vecEditPreviewPollDefinition(
    stOldPoll: &StTopicEditPoll,
    vecSubmitted: &[StTopicEditPollValue],
) -> Vec<StTopicEditPollValue> {
    let mapSubmitted = vecSubmitted
        .iter()
        .filter(|stVariant| stVariant.iVariantId != 0)
        .map(|stVariant| (stVariant.iVariantId, stVariant.sLabel.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    let mut vecVariants = stOldPoll
        .vecVariants
        .iter()
        .filter_map(|stVariant| {
            mapSubmitted
                .get(&stVariant.iId)
                .filter(|sLabel| !sLabel.is_empty())
                .map(|sLabel| StTopicEditPollValue {
                    iVariantId: stVariant.iId,
                    sLabel: (*sLabel).to_owned(),
                })
        })
        .collect::<Vec<_>>();
    vecVariants.extend(
        vecSubmitted
            .iter()
            .filter(|stVariant| stVariant.iVariantId == 0 && !stVariant.sLabel.is_empty())
            .cloned(),
    );
    vecVariants
}

#[allow(clippy::too_many_arguments)]
async fn optEditPreviewPollView(
    stState: &AppState,
    stTopic: &TopicDetail,
    stSnapshot: &crate::domain::topic::edit::StTopicEditSnapshot,
    stValues: &StEditTopicFormValues,
    optUser: &Option<UserSummary>,
    sCsrfToken: &str,
    bResultsRequested: bool,
) -> Result<Option<PollView>> {
    if !stSnapshot.bSectionPollAllowed {
        return Ok(None);
    }
    if !stValues.bPollMapPresent {
        return load_poll_view(
            stState,
            stTopic.id,
            stTopic.deleted,
            poll_is_pending(stTopic.moderate),
            stSnapshot.bExpired,
            bResultsRequested,
            optUser,
            sCsrfToken,
            &stTopic.topic_url(),
        )
        .await;
    }

    let Some(stOldPoll) = stSnapshot.optPoll.as_ref() else {
        return Ok(None);
    };
    let mapVotes = sqlx::query_as::<_, (i32, i32)>(
        "SELECT id,votes FROM polls_variants WHERE vote=$1 ORDER BY id",
    )
    .bind(stOldPoll.iId)
    .fetch_all(&stState.pool)
    .await?
    .into_iter()
    .collect::<std::collections::HashMap<_, _>>();
    let vecDefinition = vecEditPreviewPollDefinition(stOldPoll, &stValues.vecPoll);
    let iTotalVotes = vecDefinition
        .iter()
        .map(|stVariant| mapVotes.get(&stVariant.iVariantId).copied().unwrap_or(0))
        .sum::<i32>();
    let iMaxVotes = vecDefinition
        .iter()
        .map(|stVariant| mapVotes.get(&stVariant.iVariantId).copied().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let vecVariants = vecDefinition
        .into_iter()
        .map(|stVariant| {
            let iVotes = mapVotes.get(&stVariant.iVariantId).copied().unwrap_or(0);
            let iWidth = if iMaxVotes > 0 {
                320 * iVotes / iMaxVotes
            } else {
                0
            };
            PollVariantView {
                id: stVariant.iVariantId,
                label: stVariant.sLabel,
                votes: iVotes,
                pct: if iTotalVotes > 0 {
                    ((100.0 * f64::from(iVotes) / f64::from(iTotalVotes)).round()) as i32
                } else {
                    0
                },
                progress_pct: (iWidth / 16) * 16 * 100 / 320,
                progress_alt: "*".repeat(iWidth as usize),
                // PollPrepareService.preparePollPreview deliberately asks
                // PollDao for anonymous results, even for a logged-in editor.
                user_voted: false,
            }
        })
        .collect();
    let bPending = poll_is_pending(stTopic.moderate);
    let bShowResults = !bPending && (bResultsRequested || stSnapshot.bExpired);
    Ok(Some(PollView {
        voteid: stOldPoll.iId,
        multiselect: stOldPoll.bMultiSelect,
        variants: vecVariants,
        total_votes: iTotalVotes,
        // preparePollPreview passes zero, not the persisted voter count.
        total_people: 0,
        // With a submitted poll map Java's preparePollPreview is anonymous:
        // the topic tag enables the form for any authenticated viewer when
        // the topic is committed and not expired, even if this editor voted
        // before.  Deletion is not part of that tag condition.
        can_vote: optUser.is_some() && !bPending && !stSnapshot.bExpired,
        show_results: bShowResults,
        pending: bPending,
        authorized: optUser.is_some(),
        topic_url: stTopic.topic_url(),
        csrf_token: sCsrfToken.to_owned(),
    }))
}

fn sRenderEditPreviewMessage(
    sMessage: &str,
    sMarkup: &str,
    bNofollow: bool,
    sPublicUrl: &str,
    stUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> String {
    // TopicPrepareService.prepareTopicPreview calls renderTopic with
    // minimizeCut=false.  This is observably different from the comment
    // renderer: Markdown and LORCODE cuts are expanded with their topic
    // fragment containers.
    markup::render_topic_with_expanded_cut_policy_and_users(
        sMessage,
        sMarkup,
        bNofollow,
        Some(sPublicUrl),
        Some(stUsers),
    )
}

fn optEditPreviewImageView(
    pathUploadRoot: &std::path::Path,
    sName: &str,
) -> Option<TopicImageView> {
    let pathName = std::path::Path::new(sName);
    if pathName.file_name().and_then(|sValue| sValue.to_str()) != Some(sName) {
        return None;
    }
    let sStem = sName.rsplit_once('.').map_or(sName, |(sStem, _)| sStem);
    let pathRoot = pathUploadRoot.join("gallery/preview");
    let pathOriginal = pathRoot.join(sName);
    let pathMedium = pathRoot.join(format!("{sStem}-1000px.jpg"));
    let (iWidth, iHeight) = image::image_dimensions(pathOriginal).ok()?;
    let (iMediumWidth, iMediumHeight) = image::image_dimensions(pathMedium).ok()?;
    let sOriginalUrl = format!("/gallery/preview/{sName}");
    let mut vecSrcset = [500_u32, 1000, 1500, 2000]
        .into_iter()
        .filter(|iSize| iWidth > 2000 || *iSize < iWidth)
        .map(|iSize| {
            (
                format!("/gallery/preview/{sStem}-{iSize}px.jpg"),
                iSize as i32,
            )
        })
        .collect::<Vec<_>>();
    if iWidth <= 2000 {
        vecSrcset.push((sOriginalUrl.clone(), iWidth as i32));
    }
    Some(TopicImageView {
        id: 0,
        medium_url: format!("/gallery/preview/{sStem}-1000px.jpg"),
        original_url: sOriginalUrl,
        width: i32::try_from(iWidth).ok()?,
        height: i32::try_from(iHeight).ok()?,
        medium_width: i32::try_from(iMediumWidth).ok()?,
        medium_height: i32::try_from(iMediumHeight).ok()?,
        srcset: vecSrcset,
    })
}

fn vecEditPreviewImages(
    stState: &AppState,
    vecExisting: &[TopicImageView],
    vecPreviewNames: &[String],
) -> Vec<TopicImageView> {
    let mut vecImages = vecExisting.to_vec();
    vecImages.extend(vecPreviewNames.iter().filter_map(|sName| {
        optEditPreviewImageView(std::path::Path::new(&stState.config.upload_dir), sName)
    }));
    vecImages
}

pub(crate) fn stScaleUserpicDimensions(iWidth: u32, iHeight: u32) -> (i32, i32) {
    if iWidth <= 150 && iHeight <= 150 {
        return (iWidth as i32, iHeight as i32);
    }
    if iWidth >= iHeight {
        (
            150,
            ((u64::from(iHeight) * 150) / u64::from(iWidth)).max(1) as i32,
        )
    } else {
        (
            ((u64::from(iWidth) * 150) / u64::from(iHeight)).max(1) as i32,
            150,
        )
    }
}

async fn sEditPreviewUserpicHtml(
    stState: &AppState,
    stViewer: &UserSummary,
    stTopic: &crate::domain::topic::edit::StTopicEditSnapshot,
) -> Result<String> {
    let optSettings: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(stViewer.id)
            .fetch_optional(&stState.pool)
            .await?;
    let stProfile = crate::profile::ProfileSettings::from_hstore_text(optSettings);
    if !stProfile.photos {
        return Ok(String::new());
    }
    let (optPhoto, optEmail): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT photo,email FROM users WHERE id=$1")
            .bind(stTopic.iAuthorId)
            .fetch_one(&stState.pool)
            .await?;
    let mut optUrl = crate::profile::userpic_url(
        &stProfile.avatar,
        true,
        stTopic.bAuthorAnonymous,
        optPhoto.as_deref(),
        optEmail.as_deref(),
    );
    let mut stDimensions = None;
    if let (Some(sPhoto), Some(sUrl)) = (optPhoto.as_deref(), optUrl.as_deref())
        && sUrl.starts_with("/photos/")
        && std::path::Path::new(sPhoto)
            .file_name()
            .and_then(|sValue| sValue.to_str())
            == Some(sPhoto)
    {
        stDimensions = image::image_dimensions(
            std::path::Path::new(&stState.config.upload_dir)
                .join("photos")
                .join(sPhoto),
        )
        .ok()
        .map(|(iWidth, iHeight)| stScaleUserpicDimensions(iWidth, iHeight));
        if stDimensions.is_none() {
            optUrl = crate::profile::userpic_url(
                &stProfile.avatar,
                true,
                stTopic.bAuthorAnonymous,
                None,
                optEmail.as_deref(),
            );
        }
    }
    let (sUrl, iWidth, iHeight) = match optUrl {
        Some(sUrl) if sUrl.starts_with("/photos/") => {
            let (iWidth, iHeight) = stDimensions.unwrap_or((150, 150));
            (sUrl, iWidth, iHeight)
        }
        Some(sUrl) => (sUrl, 150, 150),
        None => (crate::profile::DISABLED_USERPIC.to_owned(), 1, 1),
    };
    Ok(format!(
        "<div class=\"userpic\"><img class=\"photo\" src=\"{}\" alt=\"\" width={iWidth} height={iHeight} ></div>",
        html_escape::encode_double_quoted_attribute(&sUrl)
    ))
}

#[allow(clippy::too_many_arguments)]
async fn sPrepareEditTopicCardHtml(
    stState: &AppState,
    stUser: &UserSummary,
    sCsrfToken: &str,
    stPrepared: &StPreparedTopicEdit,
    stValues: &StEditTopicFormValues,
    optTags: Option<&[String]>,
    vecExistingImages: &[TopicImageView],
    bPublish: bool,
    bResultsRequested: bool,
) -> Result<String> {
    let stSnapshot = &stPrepared.stSnapshot;
    let mut stTopic = get_topic(stState, stSnapshot.iTopicId).await?;
    let (sTitlePlain, sPreviewUrl, sPreviewLinkText) = stEditPreviewHeaderValues(
        stValues.optTitle.as_deref(),
        &stSnapshot.sStoredTitle,
        stValues.optUrl.as_deref(),
        stSnapshot.optUrl.as_deref(),
        stValues.optLinkText.as_deref(),
        stSnapshot.optLinkText.as_deref(),
    );
    // The typed edit snapshot is the controller's prepared-model source of
    // truth.  Copy its presentation fields into the unpersisted adapter too,
    // instead of accidentally relying on a second read having identical
    // values throughout the request.
    stTopic.author_frozen = stSnapshot.bAuthorFrozen;
    stTopic.group_title = stSnapshot.sGroupTitle.clone();
    stTopic.section_name = stSnapshot.sSectionTitle.clone();
    if let Some(sTitle) = stValues.optTitle.as_deref() {
        stTopic.title = crate::domain::title::sEscapeForStorage(sTitle);
    }
    stTopic.message = stValues
        .optMessage
        .clone()
        .unwrap_or_else(|| stSnapshot.sMessage.clone());
    if stSnapshot.bLinksAllowed {
        stTopic.url = stValues
            .optUrl
            .as_ref()
            .map(|_| sPreviewUrl)
            .or_else(|| stSnapshot.optUrl.clone());
        stTopic.linktext = stValues
            .optLinkText
            .as_ref()
            .map(|_| sPreviewLinkText)
            .or_else(|| stSnapshot.optLinkText.clone());
    }
    stTopic.tags = optTags.map(|vecTags| vecTags.join(","));
    stTopic.draft = stSnapshot.bDraft && !bPublish;

    let stMarkupUsers = stState
        .markup
        .stResolveBatch([(&*stTopic.message, &*stSnapshot.sMarkup)])
        .await?;
    let sTopicHtml = sRenderEditPreviewMessage(
        &stTopic.message,
        &stSnapshot.sMarkup,
        stTopic.bNofollowAuthorLinks(),
        &stState.config.public_url,
        &stMarkupUsers,
    );
    let optUser = Some(stUser.clone());
    let optPoll = optEditPreviewPollView(
        stState,
        &stTopic,
        stSnapshot,
        stValues,
        &optUser,
        sCsrfToken,
        bResultsRequested,
    )
    .await?;
    let vecImages = vecEditPreviewImages(stState, vecExistingImages, &stValues.vecUploadedImages);
    let sImagesHtml = render_topic_images_with_plain_title(
        &vecImages,
        &sTitlePlain,
        stSnapshot.bSectionImagePost,
        false,
    );
    let bActorFrozen = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT frozen_until FROM users WHERE id=$1",
    )
    .bind(stUser.id)
    .fetch_one(&stState.pool)
    .await?
    .is_some_and(|dtUntil| dtUntil > chrono::Utc::now());
    let vecTopicReactions = load_all_reactions(stState, stTopic.id, Some(stUser.id))
        .await?
        .into_iter()
        .filter(|(optCommentId, ..)| optCommentId.is_none())
        .map(|(_, sReaction, iUserId, sNick, iScore)| (sReaction, iUserId, sNick, iScore))
        .collect::<Vec<_>>();
    let bAllowReactions = reactions_allow_interact(
        &optUser,
        bActorFrozen,
        stSnapshot.bExpired,
        stTopic.author_id,
        stTopic.deleted,
        false,
    );
    let stReactions = render_reactions_widget(
        stTopic.id,
        None,
        &vecTopicReactions,
        Some(stUser.id),
        bAllowReactions,
        sCsrfToken,
    );
    let sUserpicHtml = sEditPreviewUserpicHtml(stState, stUser, stSnapshot).await?;
    sBuildTopicCardHtml(
        stState,
        &optUser,
        sCsrfToken,
        StTopicCardBuildInput {
            topic: stTopic,
            title_plain: sTitlePlain,
            topic_author_signature: stAuthorSignature(
                stSnapshot.iAuthorScore,
                stSnapshot.iAuthorMaxScore,
                !stSnapshot.bAuthorAnonymous,
                stUser.canmod,
            ),
            topic_html: sTopicHtml,
            poll: optPoll,
            images_html: sImagesHtml,
            topic_reactions: stReactions,
            userpic_html: sUserpicHtml,
            can_comment: false,
            actor_frozen: bActorFrozen,
            show_menu: false,
            enable_schema: false,
            include_canonical_extras: false,
            remote_ip: String::new(),
        },
    )
    .await
}

#[derive(Clone, Copy)]
struct CRouteTopicEditRealtimeNotifier<'a> {
    stState: &'a AppState,
}

impl TrTopicEditRealtimeNotifier for CRouteTopicEditRealtimeNotifier<'_> {
    fn vNotifyEvents(&self, vecUserIds: &[i32]) {
        self.stState
            .realtime
            .vNotifyEvents(vecUserIds.iter().copied());
    }
}

fn cTopicEditService(
    stState: &AppState,
) -> CTopicEditService<
    CTopicEditPgRepository,
    CSearchQueueSender,
    CRouteTopicEditRealtimeNotifier<'_>,
> {
    CTopicEditService::new(
        CTopicEditPgRepository::new(stState.pool.clone()),
        CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
        CRouteTopicEditRealtimeNotifier { stState },
    )
}

fn stTopicEditActor(stUser: &UserSummary) -> StTopicEditActor {
    StTopicEditActor {
        iUserId: stUser.id,
        iScore: stUser.score.unwrap_or(0),
        bModerator: stUser.canmod,
        bAdministrator: stUser.candel,
        bCorrector: stUser.corrector,
        bBlocked: stUser.blocked.unwrap_or(false),
    }
}

fn stAddActorForEdit(stUser: &UserSummary) -> StAddTopicActor {
    StAddTopicActor {
        optUserId: Some(stUser.id),
        bAnonymous: false,
        bModerator: stUser.canmod,
        bCorrector: stUser.corrector,
        bBlocked: stUser.blocked.unwrap_or(false),
        iScore: stUser.score.unwrap_or(0),
    }
}

fn sTopicEditRemoteIp(
    stState: &AppState,
    headers: &HeaderMap,
    stPeerAddress: SocketAddr,
) -> String {
    crate::security::stClientIp(
        stPeerAddress.ip(),
        headers,
        &stState.config.trusted_proxy_cidrs,
    )
    .to_string()
}

async fn stEditPublishPermission(
    stState: &AppState,
    stUser: &UserSummary,
    stPrepared: &StPreparedTopicEdit,
    sRemoteIp: &str,
) -> Result<StAddTopicPermission> {
    let stActor = stAddActorForEdit(stUser);
    let stPostingPermission = add_topic_service(stState)
        .optCheckGroup(stPrepared.stSnapshot.iGroupId, stActor, sRemoteIp)
        .await?
        .ok_or(AppError::NotFound)?;
    let stLimit = stState
        .topic_publish
        .stTopicLimitInfo(stActor, stPrepared.stSnapshot.iSectionId)
        .await?;
    Ok(stState
        .topic_publish
        .stCheckPublish(stPostingPermission, stLimit))
}

#[derive(Debug, Default)]
struct StEditTopicRenderContext {
    optTags: Option<Vec<String>>,
    bTopicCard: bool,
    bPublish: bool,
    bResultsRequested: bool,
}

const S_ALLOW_EDIT_OPTIONS: &str = "POST,GET,HEAD,OPTIONS";
const S_ALLOW_EDIT_405: &str = "POST, GET";
const S_ALLOW_COMMIT_OPTIONS: &str = "GET,HEAD,OPTIONS";
const S_ALLOW_COMMIT_405: &str = "GET";

/// Keep the legacy Spring method contract explicit.  In particular, Axum's
/// synthesized OPTIONS/405 headers use a different order and spacing.
pub fn stEditTopicRoute() -> MethodRouter<AppState> {
    get(edit_topic_form)
        .post(edit_topic)
        .options(options_edit_topic)
        .fallback(method_not_allowed_edit_topic)
        .layer(DefaultBodyLimit::max(34 * 1024 * 1024))
}

/// `/commit.jsp` only opens the shared edit form.  Confirmation itself is a
/// POST to `/edit.jsp`; accepting a POST here would be a second, non-Java
/// mutation surface.
pub fn stCommitTopicRoute() -> MethodRouter<AppState> {
    get(commit_topic_form)
        .options(options_commit_topic)
        .fallback(method_not_allowed_commit_topic)
}

fn stEditTopicMethodResponse(stStatus: StatusCode, sAllow: &'static str) -> Response {
    (
        stStatus,
        [(header::ALLOW, sAllow), (header::CONTENT_LENGTH, "0")],
    )
        .into_response()
}

async fn options_edit_topic() -> Response {
    stEditTopicMethodResponse(StatusCode::OK, S_ALLOW_EDIT_OPTIONS)
}

async fn method_not_allowed_edit_topic() -> Response {
    stEditTopicMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_ALLOW_EDIT_405)
}

async fn options_commit_topic() -> Response {
    stEditTopicMethodResponse(StatusCode::OK, S_ALLOW_COMMIT_OPTIONS)
}

async fn method_not_allowed_commit_topic() -> Response {
    stEditTopicMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_ALLOW_COMMIT_405)
}

#[allow(clippy::too_many_arguments)]
async fn stRenderEditTopic(
    stState: &AppState,
    stUser: &UserSummary,
    sCsrfToken: &str,
    stPrepared: StPreparedTopicEdit,
    stValues: StEditTopicFormValues,
    vecErrors: Vec<String>,
    bCommitForm: bool,
    sHeading: String,
    sRemoteIp: &str,
    stRenderContext: StEditTopicRenderContext,
) -> Result<Response> {
    let stTopic = &stPrepared.stSnapshot;
    let vecExistingImages = load_topic_images(stState, stTopic.iTopicId).await?;
    let bImagePost = stTopic.bSectionImagePost
        || (stTopic.bSectionImageAllowed
            && (stUser.canmod || stUser.corrector || stUser.score.unwrap_or(0) >= 50));
    let iEmptyImageSlots = if bImagePost {
        4usize.saturating_sub(vecExistingImages.len() + stValues.vecUploadedImages.len())
    } else {
        0
    };
    let iUploadedImageCount = stValues.vecUploadedImages.len();
    let vecEmptyImageSlots =
        (iUploadedImageCount..iUploadedImageCount + iEmptyImageSlots).collect();
    let stPublishPermission = if stTopic.bDraft {
        stEditPublishPermission(stState, stUser, &stPrepared, sRemoteIp).await?
    } else {
        StAddTopicPermission { optReason: None }
    };
    let (sFormatMode, sFormatTitle) = markup_form_view(&stTopic.sMarkup);
    let sFormMessage = stValues
        .optMessage
        .clone()
        .unwrap_or_else(|| stTopic.sMessage.clone());
    let sFormTitle = stValues.optTitle.clone().unwrap_or_default();
    let sFormUrl = stValues.optUrl.clone().unwrap_or_default();
    let sFormLinkText = stValues.optLinkText.clone().unwrap_or_default();
    let sFormTags = stValues.optTagsRaw.clone().unwrap_or_default();
    let optTopicCardHtml = if stRenderContext.bTopicCard {
        Some(
            sPrepareEditTopicCardHtml(
                stState,
                stUser,
                sCsrfToken,
                &stPrepared,
                &stValues,
                stRenderContext.optTags.as_deref(),
                &vecExistingImages,
                stRenderContext.bPublish,
                stRenderContext.bResultsRequested,
            )
            .await?,
        )
    } else {
        None
    };
    let mapEditorBonus = stValues
        .vecEditorBonus
        .iter()
        .cloned()
        .collect::<std::collections::HashMap<_, _>>();
    let vecEditors = stTopic
        .vecEditors
        .iter()
        .map(|stEditor| StEditEditorView {
            nick: stEditor.sNick.clone(),
            score: stEditor.iScore,
            bonus: mapEditorBonus.get(&stEditor.sNick).copied().unwrap_or(0),
            blocked: stEditor.bBlocked,
        })
        .collect();
    let vecPollVariants = stValues
        .vecPoll
        .iter()
        .filter(|stVariant| stVariant.iVariantId != 0)
        .map(|stVariant| StEditPollVariantView {
            id: stVariant.iVariantId,
            label: stVariant.sLabel.clone(),
        })
        .collect();
    let vecGroups = stTopic
        .vecGroups
        .iter()
        .map(|stGroup| StEditGroupView {
            id: stGroup.iId,
            title: stGroup.sTitle.clone(),
            selected: stValues.optChangeGroupId.unwrap_or(stTopic.iGroupId) == stGroup.iId,
        })
        .collect();
    let sPublishReason = stPublishPermission.sReason().to_owned();
    Ok(Html(
        StEditTopicTemplate {
            heading: sHeading,
            csrf_token: sCsrfToken.to_owned(),
            errors: vecErrors,
            topic_id: stTopic.iTopicId,
            last_edit: stTopic.optLastEditMillis,
            content_editable: stPrepared.stContentPermission.bPermitted(),
            tags_editable: stPrepared.stTagsPermission.bPermitted(),
            mini_editable: stPrepared.bMiniEditable(),
            links_allowed: stTopic.bLinksAllowed,
            poll_allowed: stTopic.bSectionPollAllowed,
            imagepost: bImagePost,
            existing_images: vecExistingImages,
            uploaded_images: stValues.vecUploadedImages,
            empty_image_slots: vecEmptyImageSlots,
            form_title: sFormTitle,
            form_msg: sFormMessage,
            form_url: sFormUrl,
            form_linktext: sFormLinkText,
            form_tags: sFormTags,
            form_minor: stValues.bMinor,
            poll_variants: vecPollVariants,
            new_poll: stValues.vecNewPoll,
            poll_multiselect: stValues.bMultiSelect,
            format_mode: sFormatMode,
            format_title: sFormatTitle,
            topic_card_html: optTopicCardHtml,
            draft: stTopic.bDraft,
            publish_allowed: stPublishPermission.bPermitted(),
            publish_reason: sPublishReason,
            commit_form: bCommitForm,
            groups: vecGroups,
            author_nick: stTopic.sAuthorNick.clone(),
            author_score: stTopic.iAuthorScore,
            author_blocked: stTopic.bAuthorBlocked,
            bonus: stValues.iBonus,
            editors: vecEditors,
        }
        .render()?,
    )
    .into_response())
}

pub async fn edit_topic_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = sTopicEditRemoteIp(&state, &headers, stPeerAddress);
    let stActor = stTopicEditActor(&user);
    let cService = cTopicEditService(&state);
    let stPrepared = cService
        .stPrepareEditForm(q.msgid, stActor, &sRemoteIp)
        .await?;
    let sMessage = if let Some(iRecordId) = q.from_history {
        let cHistoryService = crate::application::edit_history::CEditHistoryService::new(
            crate::infra::postgres::edit_history_repository::CEditHistoryPgRepository::new(
                state.pool.clone(),
            ),
        );
        cHistoryService
            .sRestorableTopicMessage(q.msgid, iRecordId)
            .await?
    } else {
        stPrepared.stSnapshot.sMessage.clone()
    };
    let stValues = StEditTopicFormValues::stInitial(&stPrepared, sMessage);
    stRenderEditTopic(
        &state,
        &user,
        &csrf_token,
        stPrepared,
        stValues,
        Vec::new(),
        false,
        "Редактирование".into(),
        &sRemoteIp,
        StEditTopicRenderContext::default(),
    )
    .await
}

const TOPIC_EDIT_WINDOW_DAYS: i64 = 14;

pub(crate) struct TopicEditRules {
    expired: bool,
    postscore: i32,
    commitdate: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) async fn load_topic_edit_rules(
    state: &AppState,
    topic_id: i32,
) -> Result<TopicEditRules> {
    let (expired, postscore, commitdate): (
        bool,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < CURRENT_TIMESTAMP-s.expire,
                  COALESCE(t.postscore, -9999), t.commitdate
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(TopicEditRules {
        expired,
        postscore,
        commitdate,
    })
}

fn b_topic_editable_by_author(
    topic: &TopicDetail,
    rules: &TopicEditRules,
    user: &UserSummary,
) -> bool {
    if topic.author_id != user.id {
        return false;
    }
    if topic.draft {
        return true;
    }
    if topic.moderate && topic.section_premoderated && topic.section_id != 6 {
        return false;
    }
    if !topic.moderate && (topic.sticky || topic.section_premoderated) {
        return true;
    }
    let dtBase = if topic.moderate && topic.section_id == 6 {
        rules.commitdate.unwrap_or(topic.postdate)
    } else {
        topic.postdate
    };
    chrono::Utc::now() <= dtBase + chrono::Duration::days(TOPIC_EDIT_WINDOW_DAYS)
}

pub(crate) fn b_topic_content_editable(
    topic: &TopicDetail,
    rules: &TopicEditRules,
    user: &UserSummary,
) -> bool {
    // UserPermissionService.legacyEditableFormats: only administrators may
    // edit legacy raw-HTML (markup_type=PLAIN) messages.
    if topic.markup == "PLAIN" && !user.candel {
        return false;
    }
    if user.candel {
        return true;
    }
    if rules.expired {
        return false;
    }
    if user.canmod {
        return true;
    }
    if rules.postscore == crate::domain::topic::posting::POSTSCORE_NO_COMMENTS {
        return false;
    }
    if user.corrector && topic.section_premoderated {
        return true;
    }
    b_topic_editable_by_author(topic, rules, user)
}

#[cfg(test)]
mod topic_edit_permission_tests {
    use super::*;

    fn st_user(i_id: i32, moderator: bool, administrator: bool, corrector: bool) -> UserSummary {
        UserSummary {
            id: i_id,
            nick: format!("user{i_id}"),
            name: None,
            score: Some(100),
            max_score: Some(100),
            photo: None,
            town: None,
            regdate: None,
            canmod: moderator,
            candel: administrator,
            corrector,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn st_topic() -> TopicDetail {
        TopicDetail {
            id: 10,
            title: "title".into(),
            message: "body".into(),
            markup: "MARKDOWN".into(),
            url: None,
            linktext: None,
            postdate: chrono::Utc::now(),
            lastmod: None,
            author_id: 1,
            author: "user1".into(),
            author_score: 100,
            author_blocked: false,
            author_anonymous: false,
            author_frozen: false,
            group_id: 2,
            group_title: "group".into(),
            group_urlname: "group".into(),
            section_id: 1,
            section_name: "Новости".into(),
            section_prefix: "news".into(),
            section_premoderated: true,
            comments: 0,
            deleted: false,
            sticky: false,
            resolved: None,
            tags: None,
            draft: false,
            moderate: false,
        }
    }

    fn st_rules() -> TopicEditRules {
        TopicEditRules {
            expired: false,
            postscore: crate::domain::topic::posting::POSTSCORE_UNRESTRICTED,
            commitdate: None,
        }
    }

    #[test]
    fn moderator_cannot_edit_expired_content_but_administrator_can() {
        let st_topic = st_topic();
        let mut st_rules = st_rules();
        st_rules.expired = true;
        assert!(!b_topic_content_editable(
            &st_topic,
            &st_rules,
            &st_user(2, true, false, false)
        ));
        assert!(b_topic_content_editable(
            &st_topic,
            &st_rules,
            &st_user(2, true, true, false)
        ));
    }

    #[test]
    fn corrector_obeys_no_comments_content_lock() {
        let st_topic = st_topic();
        let mut st_rules = st_rules();
        let st_corrector = st_user(2, false, false, true);
        assert!(b_topic_content_editable(
            &st_topic,
            &st_rules,
            &st_corrector
        ));
        st_rules.postscore = crate::domain::topic::posting::POSTSCORE_NO_COMMENTS;
        assert!(!b_topic_content_editable(
            &st_topic,
            &st_rules,
            &st_corrector
        ));
    }

    #[test]
    fn author_cannot_edit_committed_premoderated_news() {
        let mut st_topic = st_topic();
        let st_author = st_user(1, false, false, false);
        assert!(b_topic_content_editable(&st_topic, &st_rules(), &st_author));
        st_topic.moderate = true;
        assert!(!b_topic_content_editable(
            &st_topic,
            &st_rules(),
            &st_author
        ));
    }

    #[test]
    fn only_administrator_can_edit_legacy_html_content() {
        let mut st_topic = st_topic();
        st_topic.markup = "PLAIN".into();
        assert!(!b_topic_content_editable(
            &st_topic,
            &st_rules(),
            &st_user(2, true, false, false)
        ));
        assert!(b_topic_content_editable(
            &st_topic,
            &st_rules(),
            &st_user(2, true, true, false)
        ));
    }
}

struct StParsedEditTopicRequest {
    iTopicId: i32,
    stValues: StEditTopicFormValues,
    optTags: Option<Vec<String>>,
    bPreview: bool,
    bCommit: bool,
    bPublish: bool,
    vecErrors: Vec<String>,
}

fn bSpringFormBoolean(optValue: Option<&str>, sField: &str, vecErrors: &mut Vec<String>) -> bool {
    match optValue.map(str::to_ascii_lowercase).as_deref() {
        None | Some("false") | Some("off") | Some("no") | Some("0") => false,
        Some("true") | Some("on") | Some("yes") | Some("1") => true,
        Some(_) => {
            // Spring's CustomBooleanEditor records a binding error for every
            // other token.  It never silently treats arbitrary input as true.
            vecErrors.push(format!("Некорректное значение {sField}"));
            false
        }
    }
}

fn vecStringIndexedField(vecPairs: &[(String, String)], sPrefix: &str) -> Vec<(String, String)> {
    let sStart = format!("{sPrefix}[");
    vecPairs
        .iter()
        .filter_map(|(sKey, sValue)| {
            Some((
                sKey.strip_prefix(&sStart)?.strip_suffix(']')?.to_owned(),
                sValue.clone(),
            ))
        })
        .collect()
}

fn bJavaTopicUrl(sUrl: &str) -> bool {
    static RE_URL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(concat!(
            r"(?i)^(?:",
            r"(?:(?:https?|ftp)://(?:(?:[0-9\p{L}.-]+\.[0-9\p{L}]+)|(?:\d+\.\d+\.\d+\.\d+))(?::[0-9]+)?(?:/[^ ]*)?)",
            r"|(?:mailto:[a-z0-9_+-.]+@[0-9a-z.-]+\.[a-z]+)",
            r"|(?:news:[a-z0-9.-]+)",
            r"|(?:(?:www|ftp)\.(?:(?:[0-9a-z.-]+\.[a-z]+(?::[0-9]+)?(?:/[^ ]*)?)|(?:[a-z]+(?:/[^ ]*)?)))",
            r")$"
        ))
        .expect("URLUtil.IsUrl regex")
    });
    RE_URL.is_match(sUrl)
}

fn optInvalidXmlCharacter(sValue: &str) -> Option<String> {
    sValue
        .chars()
        .find(|cValue| {
            !matches!(
                *cValue,
                '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
            )
        })
        .map(|cValue| format!("Недопустимый XML-символ U+{:04X}", u32::from(cValue)))
}

fn stParseEditTopicRequest(vecPairs: &[(String, String)]) -> Result<StParsedEditTopicRequest> {
    use crate::form::get;

    let iTopicId = get(vecPairs, "msgid")
        .ok_or_else(|| AppError::BadParameter("Не задан msgid".into()))?
        .parse::<i32>()
        .map_err(|_| AppError::BadParameter("Некорректный msgid".into()))?;
    let bPollMapPresent = vecPairs.iter().any(|(sKey, _)| {
        sKey.strip_prefix("poll[")
            .and_then(|sSuffix| sSuffix.strip_suffix(']'))
            .is_some()
    });
    let vecOldPoll = parse_indexed_field(vecPairs, "poll")
        .into_iter()
        .map(|(iVariantId, sLabel)| StTopicEditPollValue { iVariantId, sLabel })
        .collect::<Vec<_>>();
    let mapNewPoll = parse_indexed_field(vecPairs, "newPoll")
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    let vecNewPoll = (0..POLL_NEW_VARIANT_SLOTS)
        .map(|iIndex| {
            mapNewPoll
                .get(&(iIndex as i32))
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let mut vecPoll = vecOldPoll;
    vecPoll.extend(
        vecNewPoll
            .iter()
            .cloned()
            .map(|sLabel| StTopicEditPollValue {
                iVariantId: 0,
                sLabel,
            }),
    );

    let optChangeGroupId = match get(vecPairs, "chgrp") {
        Some(sValue) => Some(
            sValue
                .parse()
                .map_err(|_| AppError::BadParameter("Некорректная группа".into()))?,
        ),
        None => None,
    };

    let mut vecErrors = Vec::new();

    let optTagsRaw = get(vecPairs, "tags").map(str::to_owned);
    let mut vecTags = optTagsRaw
        .as_deref()
        .map(crate::routes::tags::parse_tags)
        .unwrap_or_default();
    let vecBadTags = vecTags
        .iter()
        .filter(|sTag| {
            crate::domain::title::iJavaStringLength(sTag) > 32
                || !crate::routes::tags::is_good_tag(sTag)
        })
        .cloned()
        .collect::<Vec<_>>();
    vecTags.retain(|sTag| {
        crate::domain::title::iJavaStringLength(sTag) <= 32
            && crate::routes::tags::is_good_tag(sTag)
    });

    let optTitle = get(vecPairs, "title").map(str::to_owned);
    if let Some(sTitle) = optTitle.as_deref() {
        if sTitle.trim().is_empty() {
            vecErrors.push("заголовок сообщения не может быть пустым".into());
        }
        if crate::domain::title::iJavaStringLength(sTitle) > 140 {
            vecErrors.push("Слишком большой заголовок".into());
        }
        if sTitle.trim().starts_with('[') {
            vecErrors.push(
                "Не добавляйте теги в заголовки, используйте предназначенное для тегов поле ввода"
                    .into(),
            );
        }
    }
    let optMessage = get(vecPairs, "msg").map(str::to_owned);
    if let Some(sError) = optMessage.as_deref().and_then(optInvalidXmlCharacter) {
        vecErrors.push(sError);
    }
    let optUrl = get(vecPairs, "url").map(str::to_owned);
    let optLinkText = get(vecPairs, "linktext").map(str::to_owned);
    if let Some(sUrl) = optUrl.as_deref().filter(|sValue| !sValue.is_empty()) {
        if crate::domain::title::iJavaStringLength(sUrl) > 255 {
            vecErrors.push("Слишком длинный URL".into());
        }
        if !bJavaTopicUrl(sUrl) {
            vecErrors.push("Некорректный URL".into());
        }
        if optLinkText.as_deref().is_none_or(str::is_empty) {
            vecErrors.push("URL указан без текста ссылки".into());
        }
    }
    for sTag in vecBadTags {
        if crate::domain::title::iJavaStringLength(&sTag) > 32 {
            vecErrors.push(format!(
                "Слишком длинный тег: '{sTag}' (максимум 32 символов)"
            ));
        } else {
            vecErrors.push(format!("Некорректный тег: '{sTag}'"));
        }
    }
    if vecTags.len() > crate::routes::tags::MAX_TAGS_PER_TOPIC {
        vecErrors.push(format!(
            "Слишком много тегов (максимум {})",
            crate::routes::tags::MAX_TAGS_PER_TOPIC
        ));
    }
    if optTagsRaw.is_some() && vecErrors.is_empty() && vecTags.is_empty() {
        vecErrors.push("Установите теги".into());
    }

    let iBonus = match get(vecPairs, "bonus") {
        Some(sValue) => match sValue.parse::<i32>() {
            Ok(iValue) => iValue,
            Err(_) => {
                vecErrors.push("Некорректное значение bonus".into());
                3
            }
        },
        None => 3,
    };
    let mut vecEditorBonus = Vec::new();
    for (sNick, sValue) in vecStringIndexedField(vecPairs, "editorBonus") {
        match sValue.parse::<i32>() {
            Ok(iValue) => vecEditorBonus.push((sNick, iValue)),
            Err(_) => vecErrors.push("Некорректное значение editorBonus".into()),
        }
    }
    let optLastEditMillis = match get(vecPairs, "lastEdit") {
        Some(sValue) => match sValue.parse::<i64>() {
            Ok(iValue) => Some(iValue),
            Err(_) => {
                // A failed Spring property conversion remains a BindingResult
                // error even when the topic has no edit-history rows.
                vecErrors.push("Некорректное значение lastEdit".into());
                None
            }
        },
        None => None,
    };
    let bMultiSelect =
        bSpringFormBoolean(get(vecPairs, "multiselect"), "multiselect", &mut vecErrors);
    let bMinor = bSpringFormBoolean(get(vecPairs, "minor"), "minor", &mut vecErrors);

    Ok(StParsedEditTopicRequest {
        iTopicId,
        stValues: StEditTopicFormValues {
            optTitle,
            optMessage,
            optUrl,
            optLinkText,
            optTagsRaw,
            vecPoll,
            vecNewPoll,
            bPollMapPresent,
            bMultiSelect,
            bMinor,
            iBonus,
            vecEditorBonus,
            optChangeGroupId,
            optLastEditMillis,
            vecUploadedImages: parse_indexed_field(vecPairs, "uploadedImages")
                .into_iter()
                .map(|(_, sName)| sName)
                .filter(|sName| !sName.is_empty())
                .collect(),
        },
        optTags: (!vecTags.is_empty()).then_some(vecTags),
        bPreview: get(vecPairs, "preview").is_some(),
        bCommit: get(vecPairs, "commit").is_some(),
        bPublish: get(vecPairs, "publish").is_some(),
        vecErrors,
    })
}

#[cfg(test)]
mod edit_topic_binding_tests {
    use super::*;
    use axum::body::to_bytes;

    fn vecPairs(vecValues: &[(&str, &str)]) -> Vec<(String, String)> {
        vecValues
            .iter()
            .map(|(sKey, sValue)| ((*sKey).to_owned(), (*sValue).to_owned()))
            .collect()
    }

    #[test]
    fn spring_boolean_tokens_are_exact_and_case_insensitive() {
        for sValue in ["true", "TRUE", "on", "ON", "yes", "YeS", "1"] {
            let mut vecErrors = Vec::new();
            assert!(bSpringFormBoolean(Some(sValue), "minor", &mut vecErrors));
            assert!(vecErrors.is_empty(), "token={sValue}");
        }
        for optValue in [
            None,
            Some("false"),
            Some("FALSE"),
            Some("off"),
            Some("no"),
            Some("0"),
        ] {
            let mut vecErrors = Vec::new();
            assert!(!bSpringFormBoolean(optValue, "minor", &mut vecErrors));
            assert!(vecErrors.is_empty(), "token={optValue:?}");
        }
        for sValue in ["", "garbage", " true ", "2"] {
            let mut vecErrors = Vec::new();
            assert!(!bSpringFormBoolean(Some(sValue), "minor", &mut vecErrors));
            assert_eq!(vecErrors, ["Некорректное значение minor"]);
        }
    }

    #[test]
    fn malformed_boolean_and_last_edit_are_form_errors_not_true_or_ignored() {
        let stParsed = stParseEditTopicRequest(&vecPairs(&[
            ("msgid", "42"),
            ("title", "title"),
            ("msg", "body"),
            ("minor", "surprise"),
            ("multiselect", "2"),
            ("lastEdit", "not-a-number"),
        ]))
        .expect("binding result");

        assert_eq!(stParsed.iTopicId, 42);
        assert!(!stParsed.stValues.bMinor);
        assert!(!stParsed.stValues.bMultiSelect);
        assert_eq!(stParsed.stValues.optLastEditMillis, None);
        assert!(
            stParsed
                .vecErrors
                .iter()
                .any(|sError| sError.contains("lastEdit"))
        );
        assert!(
            stParsed
                .vecErrors
                .iter()
                .any(|sError| sError.contains("minor"))
        );
        assert!(
            stParsed
                .vecErrors
                .iter()
                .any(|sError| sError.contains("multiselect"))
        );
    }

    #[test]
    fn action_buttons_remain_presence_only_like_request_get_parameter() {
        let stParsed = stParseEditTopicRequest(&vecPairs(&[
            ("msgid", "42"),
            ("preview", "false"),
            ("commit", ""),
            ("publish", "0"),
        ]))
        .expect("binding result");

        assert!(stParsed.bPreview);
        assert!(stParsed.bCommit);
        assert!(stParsed.bPublish);
    }

    #[test]
    fn poll_map_presence_is_distinct_from_new_poll_slots() {
        let stMissingPollMap = stParseEditTopicRequest(&vecPairs(&[
            ("msgid", "42"),
            ("newPoll[0]", "must be ignored without poll map"),
        ]))
        .expect("binding result");
        assert!(!stMissingPollMap.stValues.bPollMapPresent);

        let stPresentPollMap =
            stParseEditTopicRequest(&vecPairs(&[("msgid", "42"), ("poll[10]", "kept")]))
                .expect("binding result");
        assert!(stPresentPollMap.stValues.bPollMapPresent);
    }

    #[test]
    fn preview_title_uses_from_edit_request_then_title_tag_pipeline() {
        assert_eq!(sEditPreviewTitle("A -- B"), "A\u{a0}— B");
        // The preview must not apply Topic.fromResultSet/makeTitle, which
        // would additionally typographize quotes after a DB round trip.
        assert_eq!(sEditPreviewTitle("\"A\" -- B"), "\"A\"\u{a0}— B");
    }

    #[test]
    fn tags_only_missing_fields_preview_falls_back_to_original_header() {
        let (sTitle, sUrl, sLinkText) = stEditPreviewHeaderValues(
            None,
            "Original -- title",
            None,
            Some("https://old.example/"),
            None,
            Some("old link"),
        );
        assert_eq!(
            sTitle,
            crate::domain::title::sTopicTitlePlainForDisplay("Original -- title")
        );
        assert_eq!(sUrl, "https://old.example/");
        assert_eq!(sLinkText, "old link");
    }

    #[test]
    fn staged_image_adapter_keeps_preview_urls_and_responsive_metadata() {
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-edit-preview-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let pathPreview = pathRoot.join("gallery/preview");
        std::fs::create_dir_all(&pathPreview).expect("preview directory");
        let sName = "preview-7-fixture.png";
        image::DynamicImage::new_rgb8(640, 480)
            .save(pathPreview.join(sName))
            .expect("preview original");
        image::DynamicImage::new_rgb8(1000, 750)
            .save(pathPreview.join("preview-7-fixture-1000px.jpg"))
            .expect("preview medium");

        let stImage = optEditPreviewImageView(&pathRoot, sName).expect("staged preview image");
        assert_eq!(stImage.id, 0);
        assert_eq!(
            stImage.original_url,
            "/gallery/preview/preview-7-fixture.png"
        );
        assert_eq!(
            stImage.medium_url,
            "/gallery/preview/preview-7-fixture-1000px.jpg"
        );
        assert_eq!((stImage.width, stImage.height), (640, 480));
        assert_eq!((stImage.medium_width, stImage.medium_height), (1000, 750));
        assert_eq!(
            stImage.srcset,
            vec![
                (
                    "/gallery/preview/preview-7-fixture-500px.jpg".to_owned(),
                    500
                ),
                ("/gallery/preview/preview-7-fixture.png".to_owned(), 640),
            ]
        );
        assert!(optEditPreviewImageView(&pathRoot, "../fixture.png").is_none());

        std::fs::remove_dir_all(pathRoot).expect("remove staged preview fixture");
    }

    #[test]
    fn submitted_poll_preview_keeps_old_order_and_appends_only_new_slots() {
        let stOldPoll = StTopicEditPoll {
            iId: 5,
            bMultiSelect: true,
            vecVariants: vec![
                crate::domain::topic::edit::StTopicEditPollVariant {
                    iId: 10,
                    sLabel: "alpha original".into(),
                },
                crate::domain::topic::edit::StTopicEditPollVariant {
                    iId: 20,
                    sLabel: "beta original".into(),
                },
            ],
        };
        let vecSubmitted = vec![
            StTopicEditPollValue {
                iVariantId: 999,
                sLabel: "unknown id".into(),
            },
            StTopicEditPollValue {
                iVariantId: 20,
                sLabel: String::new(),
            },
            StTopicEditPollValue {
                iVariantId: 10,
                sLabel: "alpha edited".into(),
            },
            StTopicEditPollValue {
                iVariantId: 0,
                sLabel: "gamma new".into(),
            },
            StTopicEditPollValue {
                iVariantId: 0,
                sLabel: String::new(),
            },
        ];
        let vecVariants = vecEditPreviewPollDefinition(&stOldPoll, &vecSubmitted);
        assert_eq!(vecVariants.len(), 2);
        assert_eq!(vecVariants[0].iVariantId, 10);
        assert_eq!(vecVariants[0].sLabel, "alpha edited");
        assert_eq!(vecVariants[1].iVariantId, 0);
        assert_eq!(vecVariants[1].sLabel, "gamma new");
    }

    #[test]
    fn preview_markup_resolves_existing_blocked_and_missing_users() {
        use crate::domain::markup::model::{StMarkupUser, StMarkupUserDirectory};

        let stUsers = StMarkupUserDirectory::stFromUsers(vec![
            StMarkupUser {
                sInputNick: "alice".into(),
                sCanonicalNick: "Alice".into(),
                bBlocked: false,
            },
            StMarkupUser {
                sInputNick: "blocked".into(),
                sCanonicalNick: "blocked".into(),
                bBlocked: true,
            },
        ]);
        let sHtml = sRenderEditPreviewMessage(
            "[user]alice[/user] [user]blocked[/user] [user]missing[/user] [cut]expanded[/cut]",
            "BBCODE_TEX",
            false,
            "https://www.linux.org.ru",
            &stUsers,
        );

        assert!(
            sHtml.contains("href=\"https://www.linux.org.ru/people/Alice/profile\""),
            "{sHtml}"
        );
        assert!(
            sHtml.contains("<s><a style=\"text-decoration: none\" href=\"https://www.linux.org.ru/people/blocked/profile\""),
            "{sHtml}"
        );
        assert!(sHtml.contains("<s>missing</s>"), "{sHtml}");
        assert!(sHtml.contains("<div id=\"cut0\">"), "{sHtml}");
        assert!(sHtml.contains("expanded"), "{sHtml}");
    }

    #[test]
    fn validator_errors_keep_java_field_order() {
        let stParsed = stParseEditTopicRequest(&vecPairs(&[
            ("msgid", "42"),
            ("title", " "),
            ("msg", "\0"),
            ("url", "not a URL"),
            ("linktext", ""),
            ("tags", "bad<tag"),
            ("bonus", "NaN"),
            ("editorBonus[alice]", "NaN"),
            ("lastEdit", "NaN"),
        ]))
        .expect("binding result");
        let vecErrors = stParsed.vecErrors;

        let iTitle = vecErrors
            .iter()
            .position(|sError| sError.contains("заголовок"))
            .expect("title error");
        let iMessage = vecErrors
            .iter()
            .position(|sError| sError.contains("XML"))
            .expect("message error");
        let iUrl = vecErrors
            .iter()
            .position(|sError| sError.contains("URL"))
            .expect("URL error");
        let iTags = vecErrors
            .iter()
            .position(|sError| sError.contains("тег"))
            .expect("tags error");
        let iBonus = vecErrors
            .iter()
            .position(|sError| sError.contains("bonus"))
            .expect("bonus error");
        let iEditorBonus = vecErrors
            .iter()
            .position(|sError| sError.contains("editorBonus"))
            .expect("editor bonus error");
        let iLastEdit = vecErrors
            .iter()
            .position(|sError| sError.contains("lastEdit"))
            .expect("last edit error");
        assert!(
            iTitle < iMessage
                && iMessage < iUrl
                && iUrl < iTags
                && iTags < iBonus
                && iBonus < iEditorBonus
                && iEditorBonus < iLastEdit
        );
    }

    #[tokio::test]
    async fn commit_user_error_is_visible_escaped_and_keeps_java_500() {
        let stResponse = stTopicEditUserErrorResponse(
            "ru.org.linux.user.UserErrorException",
            "Топик <script>уже подтвержден</script>".into(),
        );
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("legacy user-error page");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8");
        assert!(sBody.contains("Топик"));
        assert!(sBody.contains("уже подтвержден"));
        assert!(!sBody.contains("<script>уже"));
    }

    #[tokio::test]
    async fn editable_missing_title_uses_java_bad_input_500_page() {
        let stResponse = stTopicEditUserErrorResponse(
            "ru.org.linux.site.BadInputException",
            "заголовок сообщения не может быть пустым".into(),
        );
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("legacy bad-input page");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8");
        assert!(sBody.contains("ru.org.linux.site.BadInputException"));
        assert!(sBody.contains("заголовок сообщения не может быть пустым"));
    }

    #[tokio::test]
    async fn edit_and_commit_method_responses_match_spring() {
        for (stResponse, sAllow) in [
            (options_edit_topic().await, S_ALLOW_EDIT_OPTIONS),
            (options_commit_topic().await, S_ALLOW_COMMIT_OPTIONS),
        ] {
            assert_eq!(stResponse.status(), StatusCode::OK);
            assert_eq!(stResponse.headers()[header::ALLOW], sAllow);
            assert_eq!(stResponse.headers()[header::CONTENT_LENGTH], "0");
        }

        for (stResponse, sAllow) in [
            (method_not_allowed_edit_topic().await, S_ALLOW_EDIT_405),
            (method_not_allowed_commit_topic().await, S_ALLOW_COMMIT_405),
        ] {
            assert_eq!(stResponse.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(stResponse.headers()[header::ALLOW], sAllow);
            assert_eq!(stResponse.headers()[header::CONTENT_LENGTH], "0");
        }
    }

    #[test]
    fn edit_template_keeps_spring_dom_and_full_preview_hooks() {
        let sTemplate = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/edit_topic.html"
        ));
        for sNeedle in [
            "action=\"edit.jsp\"",
            "id=\"messageForm\"",
            "id=\"form_title\"",
            "id=\"form_msg\"",
            "id=\"form_multiselect\"",
            "id=\"tags\"",
            "name=\"_multiselect\"",
            "id=\"form_minor\"",
            "name=\"_minor\"",
            "name=\"lastEdit\"",
            "name=\"uploadedImages[",
            "topic_card_html",
            "{{ html|safe }}",
            "<div class=\"messages\">",
        ] {
            assert!(sTemplate.contains(sNeedle), "missing DOM hook {sNeedle}");
        }
        // EditTopicController renders the same topic tag/card as a canonical
        // topic page.  Keeping a second hand-written article here would drift
        // in author metadata, reactions, polls and responsive images.
        for sRemovedPreviewField in [
            "<article class=\"msg\"",
            "preview_poll_variants",
            "preview_poll_multiselect",
            "preview_url",
            "preview_linktext",
        ] {
            assert!(
                !sTemplate.contains(sRemovedPreviewField),
                "custom preview field remains: {sRemovedPreviewField}"
            );
        }
        let sTopicCard = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/topic_card.html"
        ));
        for sSharedHook in [
            "<article class=\"msg\"",
            "<div class=\"msg-container\">",
            "card.userpic_html",
            "card.remark_html",
            "card.committer_html",
            "card.moderator_ip_html",
            "card.moderator_user_agent_html",
            "card.topic_reactions_html",
        ] {
            assert!(
                sTopicCard.contains(sSharedHook),
                "shared topic card lacks {sSharedHook}"
            );
        }
        for sLinkedBonus in [
            "<a href=\"/people/{{ author_nick }}/profile\">{{ author_nick }}</a>): <input id=\"form_bonus\"",
            "<a href=\"/people/{{ editor.nick }}/profile\">{{ editor.nick }}</a>): <input id=\"form_editorBonus",
        ] {
            assert!(!sTemplate.contains(sLinkedBonus));
        }
    }
}

pub async fn edit_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    request: Request,
) -> Result<Response> {
    let sRemoteIp = sTopicEditRemoteIp(&state, &headers, stPeerAddress);
    let bQueryResultsRequested = request.uri().query().is_some_and(|sQuery| {
        serde_urlencoded::from_str::<Vec<(String, String)>>(sQuery)
            .is_ok_and(|vecQuery| crate::form::get(&vecQuery, "results") == Some("true"))
    });
    let (pairs, uploads) = parse_topic_request(&state, request).await?;
    let bResultsRequested =
        bQueryResultsRequested || crate::form::get(&pairs, "results") == Some("true");
    if crate::form::get(&pairs, "csrf").map(str::trim) != Some(csrf_token.trim()) {
        return Err(AppError::Forbidden);
    }
    // Spring resolves and binds @ModelAttribute and @RequestParam values
    // before entering AuthorizedOnly. Preserve that observable error order.
    let mut stRequest = stParseEditTopicRequest(&pairs)?;
    let iTopicId = stRequest.iTopicId;
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let stActor = stTopicEditActor(&user);
    let cService = cTopicEditService(&state);
    let stPreparedBeforeUpload = cService
        .stPrepareEditForm(iTopicId, stActor, &sRemoteIp)
        .await?;

    if stPreparedBeforeUpload.stContentPermission.bPermitted()
        && stRequest
            .stValues
            .optTitle
            .as_deref()
            .is_none_or(|sTitle| sTitle.trim().is_empty())
    {
        return Ok(stTopicEditUserErrorResponse(
            "ru.org.linux.site.BadInputException",
            "заголовок сообщения не может быть пустым".into(),
        ));
    }

    let vecExistingImages = load_topic_images(&state, iTopicId).await?;
    let bImagePost = stPreparedBeforeUpload.stSnapshot.bSectionImagePost
        || (stPreparedBeforeUpload.stSnapshot.bSectionImageAllowed
            && (user.canmod || user.corrector || user.score.unwrap_or(0) >= 50));
    let stPostingPermission = add_topic_service(&state)
        .optCheckGroup(
            stPreparedBeforeUpload.stSnapshot.iGroupId,
            stAddActorForEdit(&user),
            &sRemoteIp,
        )
        .await?
        .ok_or(AppError::NotFound)?;
    if bImagePost && stPostingPermission.bPermitted() {
        let iLimit = 4usize.saturating_sub(vecExistingImages.len());
        stRequest.stValues.vecUploadedImages =
            vecReusableTopicPreviews(&state, user.id, &stRequest.stValues.vecUploadedImages)
                .into_iter()
                .take(iLimit)
                .collect();
        let iUploadSlots = iLimit.saturating_sub(stRequest.stValues.vecUploadedImages.len());
        match vecStageTopicPreviews(&state, user.id, &uploads[..uploads.len().min(iUploadSlots)])
            .await
        {
            Ok(vecNames) => stRequest.stValues.vecUploadedImages.extend(vecNames),
            Err(AppError::BadRequest(sError)) => stRequest.vecErrors.push(sError),
            Err(stError) => return Err(stError),
        }
    } else {
        stRequest.stValues.vecUploadedImages.clear();
    }
    if stPreparedBeforeUpload.stSnapshot.bSectionImagePost
        && vecExistingImages.is_empty()
        && stRequest.stValues.vecUploadedImages.is_empty()
    {
        stRequest
            .vecErrors
            .push("Для этого раздела требуется как минимум одно изображение".into());
    }

    let stPublishPermission = if stPreparedBeforeUpload.stSnapshot.bDraft {
        stEditPublishPermission(&state, &user, &stPreparedBeforeUpload, &sRemoteIp).await?
    } else {
        StAddTopicPermission { optReason: None }
    };
    let optPoll = (stPreparedBeforeUpload.stSnapshot.bSectionPollAllowed
        && stRequest.stValues.bPollMapPresent)
        .then(|| stRequest.stValues.vecPoll.clone());
    let stInput = StTopicEditInput {
        optTitle: stRequest.stValues.optTitle.clone(),
        optMessage: stRequest.stValues.optMessage.clone(),
        optUrl: stRequest.stValues.optUrl.clone(),
        optLinkText: stRequest.stValues.optLinkText.clone(),
        optTags: stRequest.optTags.clone(),
        bMinor: stRequest.stValues.bMinor,
        bPreview: stRequest.bPreview,
        bCommit: stRequest.bCommit,
        bPublish: stRequest.bPublish,
        optChangeGroupId: stRequest.stValues.optChangeGroupId,
        iBonus: stRequest.stValues.iBonus,
        vecEditorBonus: stRequest.stValues.vecEditorBonus.clone(),
        optLastEditMillis: stRequest.stValues.optLastEditMillis,
        optPoll,
        bMultiSelect: stRequest.stValues.bMultiSelect,
        vecPreviewNames: stRequest.stValues.vecUploadedImages.clone(),
    };
    match cService
        .stSubmit(
            iTopicId,
            stActor,
            &sRemoteIp,
            stInput,
            stRequest.vecErrors,
            stPublishPermission.bPermitted(),
            stPublishPermission.sReason(),
            &state.config.upload_dir,
        )
        .await?
    {
        EnTopicEditOutcome::Render {
            stPrepared,
            vecErrors,
            bCommitForm,
            sHeading,
        } => {
            stRenderEditTopic(
                &state,
                &user,
                &csrf_token,
                *stPrepared,
                stRequest.stValues,
                vecErrors,
                bCommitForm,
                sHeading,
                &sRemoteIp,
                StEditTopicRenderContext {
                    optTags: stRequest.optTags.clone(),
                    bTopicCard: true,
                    bPublish: stRequest.bPublish,
                    bResultsRequested,
                },
            )
            .await
        }
        EnTopicEditOutcome::Applied {
            sRedirectUrl,
            bModeratedConfirmation,
            ..
        } => {
            for sPreviewName in &stRequest.stValues.vecUploadedImages {
                vDeleteTopicPreview(&state, sPreviewName).await;
            }
            if bModeratedConfirmation {
                Ok(Html(
                    ModeratedTopicTemplate {
                        topic_url: sRedirectUrl,
                    }
                    .render()?,
                )
                .into_response())
            } else {
                Ok((StatusCode::FOUND, [(header::LOCATION, sRedirectUrl)]).into_response())
            }
        }
    }
}

/// Matches GroupPermissionService.DeletePeriod: an author may delete their
/// own (non-draft, non-premoderated-and-committed) topic for 3 days after
/// posting, and only while it has no comments. Moderators bypass this.
const TOPIC_DELETE_WINDOW_HOURS: i64 = 72;

fn b_topic_deletable(
    meta: &TopicDeleteMeta,
    user: &UserSummary,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> bool {
    let bDeletableByAuthor = meta.author_id == user.id
        && (meta.draft
            || (!(meta.premoderated && meta.commited)
                && meta.comment_count == 0
                && dtNow < meta.postdate + chrono::Duration::hours(TOPIC_DELETE_WINDOW_HOURS)));
    if user.candel || bDeletableByAuthor {
        true
    } else if user.canmod {
        !meta.premoderated
            || !meta.commited
            || meta
                .postdate
                .with_timezone(&chrono::Local)
                .checked_add_months(chrono::Months::new(1))
                .is_some_and(|dtDeadline| dtDeadline.with_timezone(&chrono::Utc) > dtNow)
    } else {
        false
    }
}

struct TopicDeleteMeta {
    author_id: i32,
    deleted: bool,
    postdate: chrono::DateTime<chrono::Utc>,
    draft: bool,
    premoderated: bool,
    commited: bool,
    comment_count: i64,
}

type TyTopicDeleteRow = (
    i32,
    bool,
    chrono::DateTime<chrono::Utc>,
    bool,
    bool,
    bool,
    i64,
);

pub(crate) async fn b_user_slow_mode_restricted(
    state: &AppState,
    user: &UserSummary,
) -> Result<bool> {
    let stActor = crate::domain::topic::posting::StAddTopicActor {
        optUserId: Some(user.id),
        bAnonymous: false,
        bModerator: user.canmod,
        bCorrector: user.corrector,
        bBlocked: user.blocked.unwrap_or(false),
        iScore: user.score.unwrap_or(0),
    };
    let cService = CAddTopicService::new(CAddTopicPgRepository::new(state.pool.clone()));
    cService.bSlowModeRestricted(stActor).await
}

/// GroupPermissionService.canViewAllDeletedTopics: a listing-level "show me
/// deleted topics too" gate, distinct from (and much looser than) the
/// per-topic `ViewDeletedScore=200` in `check_topic_viewable` - any
/// authorized, non-frozen user with score>=50 qualifies, not just
/// moderators. Java additionally rejects users restricted by SlowModeChecker.
pub(crate) async fn can_view_all_deleted_topics(
    state: &AppState,
    user: &Option<UserSummary>,
) -> Result<bool> {
    const CAN_VIEW_ALL_DELETED_SCORE: i32 = 50;
    let Some(user) = user else {
        return Ok(false);
    };
    // Java's canViewAllDeletedTopics has no isModerator special-case at
    // all - the score+frozen check applies uniformly, moderators included.
    if user.score.unwrap_or(0) < CAN_VIEW_ALL_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Ok(false);
    }
    Ok(!b_user_slow_mode_restricted(state, user).await?)
}

/// TopicPermissionService.allowViewAllDeletedComments: the `?deleted=`
/// gate on a topic's own page - narrower than `can_view_all_deleted_topics`
/// (score>=200, not 50) but *does* bypass for moderators, unlike that one.
/// SlowModeChecker is evaluated after the topic and score-loss gates, as in
/// the Java service.
pub(crate) async fn allow_view_all_deleted_comments(
    state: &AppState,
    topic_id: i32,
    user: &Option<UserSummary>,
) -> Result<bool> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Ok(true);
    }
    const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
    const POSTSCORE_NO_COMMENTS: i32 = 10001;
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    let Some((expired, draft, postscore)): Option<(bool, bool, i32)> = sqlx::query_as(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, COALESCE(t.draft,false), COALESCE(t.postscore, -9999)
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(false);
    };
    let topic_forbidden = expired
        || draft
        || matches!(
            postscore,
            POSTSCORE_MODERATORS_ONLY | POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS
        );
    if topic_forbidden {
        return Ok(false);
    }
    let Some(user) = user else {
        return Ok(false);
    };
    if user.score.unwrap_or(0) < VIEW_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Ok(false);
    }
    let score_loss: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE((SELECT sum(-bonus) FROM del_info JOIN comments ON comments.id=del_info.msgid
             WHERE bonus IS NOT NULL AND bonus<>0 AND comments.userid<>2 AND comments.deleted AND topic=$1), 0)::int"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(score_loss < 150 && !b_user_slow_mode_restricted(state, user).await?)
}

/// TopicPermissionService.ViewDeletedScore/ViewAfterDeleteDays/TopicMaxWarnings.
const VIEW_DELETED_SCORE: i32 = 200;
const VIEW_AFTER_DELETE_DAYS: i64 = 14;
const TOPIC_MAX_WARNINGS: i32 = 2;

/// TopicPermissionService.checkView: whether `user` may view this specific
/// topic given its deleted/draft/expired/open-warnings state. Moderators
/// always pass (mirrors `!session.moderator` guarding the whole body in
/// Java). Used both for the standalone topic view and, transitively, for
/// anything that needs the same "can view a deleted topic" rule (reactions
/// viewer, forum/group `showDeleted` gate).
pub(crate) async fn check_topic_viewable(
    state: &AppState,
    topic_id: i32,
    user: &Option<UserSummary>,
) -> Result<()> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Ok(());
    }
    let row: Option<(bool, bool, bool, i32, i32, bool)> = sqlx::query_as(
        r#"SELECT t.deleted, COALESCE(t.draft,false),
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired,
                  t.userid, t.open_warnings, u.canmod
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           JOIN users u ON u.id=t.userid
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((deleted, draft, expired, author_id, open_warnings, author_is_moderator)) = row else {
        return Err(AppError::NotFound);
    };

    let view_by_author = user.as_ref().map(|u| u.id == author_id).unwrap_or(false);

    if deleted {
        if expired {
            return Err(AppError::NotFound);
        }
        if user.is_none() {
            return Err(AppError::NotFound);
        }
        if !view_by_author {
            let current = user.as_ref().unwrap();
            let deldate: Option<chrono::DateTime<chrono::Utc>> =
                sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1")
                    .bind(topic_id)
                    .fetch_optional(&state.pool)
                    .await?
                    .flatten();
            let delete_expired = deldate
                .map(|d| d < chrono::Utc::now() - chrono::Duration::days(VIEW_AFTER_DELETE_DAYS))
                .unwrap_or(true);
            if delete_expired {
                return Err(AppError::NotFound);
            }
            let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
                sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
                    .bind(current.id)
                    .fetch_optional(&state.pool)
                    .await?
                    .flatten();
            if frozen_until
                .map(|u| u > chrono::Utc::now())
                .unwrap_or(false)
            {
                return Err(AppError::Forbidden);
            }
            if current.score.unwrap_or(0) < VIEW_DELETED_SCORE {
                return Err(AppError::NotFound);
            }
            if author_is_moderator {
                return Err(AppError::NotFound);
            }
        }
    }

    if draft {
        if expired {
            return Err(AppError::NotFound);
        }
        if !view_by_author {
            return Err(AppError::NotFound);
        }
    }

    if user.is_none() && open_warnings > TOPIC_MAX_WARNINGS {
        return Err(AppError::NotFound);
    }

    Ok(())
}

async fn load_topic_delete_meta(state: &AppState, msgid: i32) -> Result<TopicDeleteMeta> {
    let row: TyTopicDeleteRow = sqlx::query_as(
        r#"SELECT t.userid, t.deleted, t.postdate, COALESCE(t.draft,false), s.moderate,
                  t.moderate, t.stat1::bigint
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(TopicDeleteMeta {
        author_id: row.0,
        deleted: row.1,
        postdate: row.2,
        draft: row.3,
        premoderated: row.4,
        commited: row.5,
        comment_count: row.6,
    })
}

pub async fn list_topics(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
    offset: i64,
    limit: i64,
) -> Result<Vec<TopicSummary>> {
    topic_service(state)
        .vecListTopics(section, group, offset, limit)
        .await
}

async fn list_topics_filtered(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
    offset: i64,
    limit: i64,
    no_talks: bool,
    tech: bool,
) -> Result<Vec<TopicSummary>> {
    topic_service(state)
        .vecListTopicsFiltered(section, group, offset, limit, no_talks, tech)
        .await
}

pub async fn get_topic(state: &AppState, id: i32) -> Result<TopicDetail> {
    topic_service(state).stGetTopic(id).await
}

fn topic_service(state: &AppState) -> CTopicService<CTopicPgRepository> {
    CTopicService::new(CTopicPgRepository::new(state.pool.clone()))
}

fn add_topic_service(state: &AppState) -> CAddTopicService<CAddTopicPgRepository> {
    CAddTopicService::new(CAddTopicPgRepository::new(state.pool.clone()))
}

fn stPostingActor(stIdentity: &crate::application::auth::StPostingIdentity) -> StAddTopicActor {
    let stUser = &stIdentity.stUser;
    StAddTopicActor {
        optUserId: Some(stUser.id),
        bAnonymous: !stIdentity.bAuthorized,
        bModerator: stUser.canmod,
        bCorrector: stUser.corrector,
        bBlocked: stUser.blocked.unwrap_or(false),
        iScore: stUser.score.unwrap_or(0),
    }
}

/// PollDao.createPoll/updatePoll unified into one helper: creates the
/// topic's poll row on first call, then on every call reconciles
/// `polls_variants` against the submitted (variant_id, label) pairs -
/// `variant_id==0` inserts a new variant, an existing id with an empty
/// label deletes it, an existing id with a non-empty label updates it.
/// `variant_id` is scoped to `vote=voteid` in every UPDATE/DELETE so a
/// forged id from another poll can't be touched.
async fn save_poll(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    topic_id: i32,
    multiselect: bool,
    variant_ids: &[i32],
    labels: &[String],
) -> Result<()> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM polls WHERE topic=$1")
        .bind(topic_id)
        .fetch_optional(&mut **tx)
        .await?;
    let voteid = match existing {
        Some(id) => {
            sqlx::query("UPDATE polls SET multiselect=$1 WHERE id=$2")
                .bind(multiselect)
                .bind(id)
                .execute(&mut **tx)
                .await?;
            id
        }
        None => {
            let id: i32 = sqlx::query_scalar("SELECT nextval('vote_id')::int")
                .fetch_one(&mut **tx)
                .await?;
            sqlx::query("INSERT INTO polls(id, multiselect, topic) VALUES($1,$2,$3)")
                .bind(id)
                .bind(multiselect)
                .bind(topic_id)
                .execute(&mut **tx)
                .await?;
            id
        }
    };
    for (variant_id, label) in variant_ids.iter().zip(labels.iter()) {
        let label = label.trim();
        if *variant_id == 0 {
            if !label.is_empty() {
                sqlx::query("INSERT INTO polls_variants(id, vote, label) VALUES(nextval('votes_id'), $1, $2)").bind(voteid).bind(label).execute(&mut **tx).await?;
            }
        } else if label.is_empty() {
            sqlx::query("DELETE FROM polls_variants WHERE id=$1 AND vote=$2")
                .bind(variant_id)
                .bind(voteid)
                .execute(&mut **tx)
                .await?;
        } else {
            sqlx::query("UPDATE polls_variants SET label=$1 WHERE id=$2 AND vote=$3")
                .bind(label)
                .bind(variant_id)
                .bind(voteid)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

/// TopicService.sendEvents/UserEventDao.insertTopicNotification. This runs
/// inside the topic write transaction and records `topic_users_notified`, so
/// later edits do not emit duplicate mention or favorite-tag notifications.
async fn notify_topic_users_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    topic_id: i32,
    author_id: i32,
    message: &str,
    message_markup: &str,
    bIncludeTagEvents: bool,
) -> Result<Vec<i32>> {
    let mentioned_nicks = markup::extract_mentions(message, message_markup);
    let mut notified: Vec<i32> = if mentioned_nicks.is_empty() {
        vec![]
    } else {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE u.nick = ANY($1) AND u.id <> $2
                 AND ($4 OR NOT COALESCE(u.blocked,false))
                 AND NOT EXISTS (
                     SELECT 1 FROM topic_users_notified tun
                     WHERE tun.topic=$3 AND tun.userid=u.id
                 )
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(&mentioned_nicks)
        .bind(author_id)
        .bind(topic_id)
        .bind(markup::mentions_include_blocked_users(message_markup))
        .fetch_all(&mut **tx)
        .await?
    };
    for &mentioned_id in &notified {
        sqlx::query(
            "INSERT INTO topic_users_notified(topic,userid) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(topic_id)
        .bind(mentioned_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'REF',false,$2)",
        )
        .bind(mentioned_id)
        .bind(topic_id)
        .execute(&mut **tx)
        .await?;
    }

    let tag_favoriters: Vec<i32> = if bIncludeTagEvents {
        sqlx::query_scalar(
            r#"SELECT DISTINCT ut.user_id FROM user_tags ut
               JOIN tags tg ON tg.tagid=ut.tag_id
               WHERE tg.msgid=$1 AND ut.is_favorite AND ut.user_id<>$2
                 AND NOT ut.user_id=ANY($3)
                 AND NOT EXISTS (
                     SELECT 1 FROM topic_users_notified tun
                     WHERE tun.topic=$1 AND tun.userid=ut.user_id
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM ignore_list il
                     WHERE il.userid=ut.user_id AND il.ignored=$2
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM user_tags ignored_tag
                     JOIN tags topic_tag ON topic_tag.tagid=ignored_tag.tag_id
                     WHERE ignored_tag.user_id=ut.user_id
                       AND NOT ignored_tag.is_favorite
                       AND topic_tag.msgid=$1
                 )"#,
        )
        .bind(topic_id)
        .bind(author_id)
        .bind(&notified)
        .fetch_all(&mut **tx)
        .await?
    } else {
        Vec::new()
    };
    for &tag_userid in &tag_favoriters {
        sqlx::query(
            "INSERT INTO topic_users_notified(topic,userid) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(topic_id)
        .bind(tag_userid)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'TAG',false,$2)",
        )
        .bind(tag_userid)
        .bind(topic_id)
        .execute(&mut **tx)
        .await?;
    }
    notified.extend(tag_favoriters);

    if !notified.is_empty() {
        notified.sort_unstable();
        notified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&notified)
            .execute(&mut **tx)
            .await?;
    }
    Ok(notified)
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    match uri.path().trim_start_matches('/').split('/').next()? {
        "forum" => Some("forum"),
        "news" => Some("news"),
        "articles" => Some("articles"),
        "gallery" => Some("gallery"),
        "polls" => Some("polls"),
        _ => None,
    }
}

fn section_title(section: &str) -> &'static str {
    match section {
        "forum" => "Форум",
        "news" => "Новости",
        "articles" => "Статьи",
        "gallery" => "Галерея",
        "polls" => "Опросы",
        _ => "Темы",
    }
}

/// GroupPermissionService.isUndeletable: an administrator can always restore
/// a deleted topic; a plain moderator can do so while it is live, or for 14
/// days after deletion if the topic has already expired.
async fn bTopicUndeletable(
    state: &AppState,
    stMeta: &TopicDeleteMeta,
    stUser: &UserSummary,
    iTopicId: i32,
) -> Result<bool> {
    if !stMeta.deleted || !stUser.canmod {
        return Ok(false);
    }
    if stUser.candel || !crate::routes::comments::is_topic_expired(state, iTopicId).await? {
        return Ok(true);
    }
    let optDeleteDate: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1")
            .bind(iTopicId)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    Ok(optDeleteDate
        .is_some_and(|dtValue| dtValue > chrono::Utc::now() - chrono::Duration::days(14)))
}

/// EditTopicChecker.checkCommit: moderators or correctors may commit a
/// news topic, but a corrector may not commit their own - moderators have
/// no such restriction.
fn check_commit_allowed(user: &UserSummary, topic_author_id: i32) -> Result<()> {
    if !user.canmod && !user.corrector {
        return Err(AppError::Forbidden);
    }
    if user.corrector && user.id == topic_author_id {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub async fn commit_topic_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = sTopicEditRemoteIp(&state, &headers, stPeerAddress);
    let cService = cTopicEditService(&state);
    let stPrepared = match cService
        .stPrepareCommitForm(q.msgid, stTopicEditActor(&user), &sRemoteIp)
        .await
    {
        Ok(stPrepared) => stPrepared,
        // UserErrorException is rendered by the Java global resolver as its
        // common error page with HTTP 500 and a visible, escaped message.
        Err(AppError::BadRequest(sMessage)) => {
            return Ok(stTopicEditUserErrorResponse(
                "ru.org.linux.user.UserErrorException",
                sMessage,
            ));
        }
        Err(stError) => return Err(stError),
    };
    let sMessage = if let Some(iRecordId) = q.from_history {
        let cHistoryService = crate::application::edit_history::CEditHistoryService::new(
            crate::infra::postgres::edit_history_repository::CEditHistoryPgRepository::new(
                state.pool.clone(),
            ),
        );
        cHistoryService
            .sRestorableTopicMessage(q.msgid, iRecordId)
            .await?
    } else {
        stPrepared.stSnapshot.sMessage.clone()
    };
    let stValues = StEditTopicFormValues::stInitial(&stPrepared, sMessage);
    stRenderEditTopic(
        &state,
        &user,
        &csrf_token,
        stPrepared,
        stValues,
        Vec::new(),
        true,
        "Подтверждение".into(),
        &sRemoteIp,
        StEditTopicRenderContext::default(),
    )
    .await
}
