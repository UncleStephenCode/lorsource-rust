use crate::{
    application::topic::{CTopicService, posting::CAddTopicService},
    auth::CurrentUser,
    domain::topic::{
        posting::{StAddTopicActor, StAddTopicPermission, StTopicLimitInfo},
        repository::{StEditTopic, StNewTopic},
    },
    error::{AppError, Result},
    infra::postgres::{
        add_topic_repository::CAddTopicPgRepository, topic_repository::CTopicPgRepository,
    },
    markup,
    models::{CommentItem, Group, PagerQuery, TagItem, TopicDetail, TopicSummary, UserSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    extract::{ConnectInfo, FromRequest, Multipart, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, Uri, header, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    news: Vec<NewsTopicView>,
    pager: Pager,
    main_page: bool,
    tracker_layout: bool,
    navigation: Option<TopicListNavigation>,
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
    poll: Option<TopicSummary>,
    articles: Vec<TopicSummary>,
    top_topics: Vec<TopicSummary>,
    gallery: Vec<GalleryBoxItem>,
    tags: Vec<TagItem>,
    show_gallery_on_main: bool,
}

struct GalleryBoxItem {
    topic: TopicSummary,
    image_url: String,
    image_srcset: String,
    image_width: i32,
    image_height: i32,
    image_padding_percent: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct QuickGroupLink {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TopicListNavigation {
    pub(crate) section_url: Option<String>,
    pub(crate) archive_url: Option<String>,
    pub(crate) rss_url: Option<String>,
    pub(crate) add_url: Option<String>,
    pub(crate) add_reason: String,
    pub(crate) moderator_group_id: Option<i32>,
    pub(crate) quick_groups: Vec<QuickGroupLink>,
    pub(crate) all_groups_selected: bool,
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
    topic_html: String,
    comments: Vec<CommentView>,
    /// Non-empty only outside thread/deleted mode, when there's more than
    /// one page of comments (TopicController.buildPages).
    pages: Vec<CommentPageLink>,
    thread_mode: bool,
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
    poll: Option<PollView>,
    images_html: String,
    topic_reactions_html: String,
    topic_show_reactions_link: bool,
    comment_format_mode: String,
    comment_format_title: String,
    can_comment: bool,
    anonymous_comment_form: bool,
    require_comment_captcha: bool,
    captcha_site_key: String,
    realtime_bootstrap_html: String,
    related_topics: Vec<Vec<crate::search_index::StSimilarTopic>>,
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
        assert_eq!(
            format!("{:x}", Sha256::digest(arrScript)),
            "09fae4a64dbdfee232042ae76eb3e03f1521b9ecef352c6e8a1b6656c2a55c64"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(arrRealtime)),
            "1665374fa67a2fc27681c6bb9ac92017ef2dbc78539cf947bfb050c70ddfb10a"
        );
    }
}

#[derive(Debug, Clone)]
struct CommentView {
    item: CommentItem,
    html: String,
    reactions_html: String,
    show_reactions_link: bool,
    can_edit: bool,
}

/// poll-form.tag rendered server-side: a topic's poll (if any), with vote
/// counts/percentages and whether the current viewer may still vote.
/// `can_vote` doesn't pre-check expiry (Topic.expired isn't loaded by
/// `get_topic` here) - `/vote.jsp` itself still rejects an expired poll,
/// so a stale "Голосовать" button just surfaces that error instead of
/// silently vanishing a beat early.
#[derive(Debug, Clone)]
struct PollView {
    voteid: i32,
    multiselect: bool,
    variants: Vec<PollVariantView>,
    total_votes: i32,
    total_people: i64,
    can_vote: bool,
    show_results: bool,
    pending: bool,
    authorized: bool,
}

#[derive(Debug, Clone)]
struct PollVariantView {
    id: i32,
    label: String,
    votes: i32,
    pct: i32,
    progress_pct: i32,
    user_voted: bool,
}

/// PreparedImage-compatible view of any image attached to a topic.
pub(crate) struct TopicImageView {
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
    pub(crate) show_group: bool,
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
    title: &str,
    imagepost: bool,
    news: bool,
) -> String {
    match images {
        [] => String::new(),
        [image] => render_single_image(image, title, imagepost, news),
        _ => render_image_slider(images, title, news),
    }
}

#[cfg(test)]
mod image_view_tests {
    use super::*;

    fn image(id: i32) -> TopicImageView {
        TopicImageView {
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
}

pub(crate) async fn prepare_news_topics(
    state: &AppState,
    topics: Vec<TopicSummary>,
    show_group: bool,
) -> Result<Vec<NewsTopicView>> {
    let mut prepared = Vec::with_capacity(topics.len());
    for topic in topics {
        type TyNewsTopicRow = (String, String, Option<String>, Option<String>);
        let row: Option<TyNewsTopicRow> = sqlx::query_as(
            "SELECT m.message, m.markup::text, t.linktext, g.image FROM msgbase m JOIN topics t ON t.id=m.id JOIN groups g ON g.id=t.groupid WHERE m.id=$1",
        )
        .bind(topic.id)
        .fetch_optional(&state.pool)
        .await?;
        let (message, message_markup, linktext, group_image) =
            row.unwrap_or_else(|| (String::new(), "BBCODE_TEX".into(), None, None));
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
        prepared.push(NewsTopicView {
            topic_html: markup::render_message_with_markup(&message, Some(&message_markup), None),
            images_html,
            group_image_url,
            linktext: linktext
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Подробности".to_string()),
            topic,
            show_group,
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
    let divisor = if total_people > 0 {
        total_people as i32
    } else {
        total_votes
    };
    let max_votes = rows.iter().map(|row| row.2).max().unwrap_or(0);
    let variants = rows
        .into_iter()
        .map(|(id, label, votes, selected)| PollVariantView {
            id,
            label,
            votes,
            pct: if divisor > 0 {
                ((100.0 * f64::from(votes) / f64::from(divisor)).round()) as i32
            } else {
                0
            },
            progress_pct: if max_votes > 0 {
                ((320 * votes / max_votes) / 16) * 16 * 100 / 320
            } else {
                0
            },
            user_voted: selected,
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
    }))
}

fn poll_is_pending(topic_committed: bool) -> bool {
    !topic_committed
}

#[cfg(test)]
mod poll_moderation_semantics_tests {
    use super::poll_is_pending;

    #[test]
    fn poll_is_pending_until_topics_moderate_is_true() {
        assert!(poll_is_pending(false));
        assert!(!poll_is_pending(true));
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
    poll_allowed: bool,
    image_allowed: bool,
    image_required: bool,
    additional_image_rows: Vec<()>,
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
    pub id: Option<i32>,
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
    pub variant_id: Vec<i32>,
    pub multiselect: Option<String>,
    pub nick: Option<String>,
    pub password: Option<String>,
    pub captcha_response: Option<String>,
    pub allow_anonymous: Option<String>,
    pub uploaded_images: Vec<String>,
}

/// `axum::Form` can't deserialize the repeated `poll`/`variant_id` keys into
/// `Vec` fields (see `crate::form`), so this form is parsed from the raw
/// body by hand instead.
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
    let (poll, variant_id) = if !indexed_poll.is_empty() || !new_poll.is_empty() {
        let mut ids = indexed_poll.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let mut labels = indexed_poll
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>();
        ids.extend(std::iter::repeat_n(0, new_poll.len()));
        labels.extend(new_poll.into_iter().map(|(_, label)| label));
        (labels, ids)
    } else {
        // Accept the first Rust port's flattened fields as a compatibility
        // fallback, while every generated form uses Java's indexed names.
        (
            get_all(pairs, "poll")
                .into_iter()
                .map(str::to_string)
                .collect(),
            get_all(pairs, "variant_id")
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .collect(),
        )
    };
    Ok(TopicForm {
        id: get(pairs, "msgid")
            .or_else(|| get(pairs, "id"))
            .and_then(|v| v.parse().ok()),
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
        variant_id,
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
        assert_eq!(form.variant_id, [0, 1]);
        assert!(form.multiselect.is_some());
    }

    #[test]
    fn parses_java_edit_topic_poll_contract_without_group() {
        let form = parse_topic_form(&pairs(&[
            ("msgid", "42"),
            ("title", "Опрос"),
            ("msg", "Текст"),
            ("tags", "lor"),
            ("poll[17]", "Существующий"),
            ("newPoll[0]", "Новый"),
            ("multiselect", "on"),
        ]))
        .unwrap();
        assert_eq!(form.id, Some(42));
        assert_eq!(form.group, 0);
        assert_eq!(form.poll, ["Существующий", "Новый"]);
        assert_eq!(form.variant_id, [17, 0]);
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
        assert_eq!(form.variant_id, [12, 0]);
    }
}

pub async fn index(
    State(state): State<AppState>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    let _ = q;
    let show_gallery_on_main = match &current_user {
        Some(user) => {
            let settings_text: Option<String> =
                sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                    .bind(user.id)
                    .fetch_optional(&state.pool)
                    .await?
                    .flatten();
            crate::profile::ProfileSettings::from_hstore_text(settings_text).main_gallery
        }
        None => crate::profile::ProfileSettings::default().main_gallery,
    };
    let all_topics = if show_gallery_on_main {
        let mut topics = Vec::new();
        for section in ["news", "gallery", "polls", "articles"] {
            topics.extend(list_topics(&state, Some(section), None, 0, 30).await?);
        }
        topics.sort_by(|left, right| {
            let left_date = left.lastmod.as_ref().unwrap_or(&left.postdate);
            let right_date = right.lastmod.as_ref().unwrap_or(&right.postdate);
            right
                .sticky
                .cmp(&left.sticky)
                .then_with(|| right_date.cmp(left_date))
        });
        topics.truncate(30);
        topics
    } else {
        list_topics(&state, Some("news"), None, 0, 30).await?
    };
    let news =
        prepare_news_topics(&state, all_topics.iter().take(10).cloned().collect(), true).await?;
    let brief = all_topics.iter().skip(10).cloned().collect();
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
    let poll = if show_gallery_on_main {
        None
    } else {
        list_topics(&state, Some("polls"), None, 0, 1)
            .await?
            .into_iter()
            .next()
    };
    let articles = if show_gallery_on_main {
        Vec::new()
    } else {
        list_topics(&state, Some("articles"), None, 0, 7).await?
    };
    let top_topics = all_topics.iter().take(10).cloned().collect();
    let mut gallery = Vec::new();
    if !show_gallery_on_main {
        for topic in list_topics(&state, Some("gallery"), None, 0, 12).await? {
            if let Some(image) = load_topic_images(&state, topic.id)
                .await?
                .into_iter()
                .next()
            {
                let srcset = image_srcset(&image);
                let padding_percent =
                    100.0 * f64::from(image.medium_height) / f64::from(image.medium_width);
                gallery.push(GalleryBoxItem {
                    topic,
                    image_url: image.medium_url,
                    image_srcset: srcset,
                    image_width: image.medium_width,
                    image_height: image.medium_height,
                    image_padding_percent: padding_percent,
                });
                if gallery.len() == 3 {
                    break;
                }
            }
        }
    }
    let tags = sqlx::query_as::<_, TagItem>("SELECT value,counter FROM tags_values WHERE counter>0 ORDER BY counter DESC,lower(value) LIMIT 25").fetch_all(&state.pool).await?;
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
            poll,
            articles,
            top_topics,
            gallery,
            tags,
            show_gallery_on_main,
        }
        .render()?,
    ))
}

pub async fn lenta(
    State(state): State<AppState>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some("forum"), None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    let navigation = build_topic_list_navigation(&state, "forum", None, &current_user).await?;
    Ok(Html(
        IndexTemplate {
            title: "Форум / лента".into(),
            topics,
            news,
            pager,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
        }
        .render()?,
    ))
}

pub async fn section_topics(
    State(state): State<AppState>,
    uri: Uri,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    let navigation = build_topic_list_navigation(&state, section, None, &current_user).await?;
    Ok(Html(
        IndexTemplate {
            title: section_title(section).to_string(),
            topics,
            news,
            pager,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
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
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(
        &state,
        Some(section),
        Some(&group),
        pager.offset,
        pager.limit,
    )
    .await?;
    let selected = crate::routes::groups::find_group(&state, &group).await?;
    if selected.section_prefix != section {
        return Err(AppError::NotFound);
    }
    let news = prepare_news_topics(&state, topics.clone(), false).await?;
    let navigation =
        build_topic_list_navigation(&state, section, Some(&selected), &current_user).await?;
    Ok(Html(
        IndexTemplate {
            title: format!("{} «{}»", section_title(section), selected.title),
            topics,
            news,
            pager,
            main_page: false,
            tracker_layout: false,
            navigation: Some(navigation),
        }
        .render()?,
    ))
}

pub async fn legacy_show_topics(
    State(state): State<AppState>,
    Query(q): Query<PagerQuery>,
    CurrentUser(_current_user): CurrentUser,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    Ok(Html(
        IndexTemplate {
            title: "show-topics.jsp".into(),
            topics,
            news,
            pager,
            main_page: false,
            tracker_layout: false,
            navigation: None,
        }
        .render()?,
    ))
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
    messages: Vec<TopicSummary>,
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

async fn build_topic_list_navigation(
    state: &AppState,
    section_prefix: &str,
    selected_group: Option<&Group>,
    user: &Option<UserSummary>,
) -> Result<TopicListNavigation> {
    let (section_id, section_restriction): (i32, i32) = sqlx::query_as(
        r#"SELECT id, COALESCE(restrict_topics,-9999) FROM sections WHERE CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END=$1"#,
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
    Ok(TopicListNavigation {
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
) -> Result<Html<String>> {
    let section: Option<ViewAllSection> = if let Some(sid) = q.section.filter(|&id| id != 0) {
        let sql = format!(
            "SELECT s.id, s.name, COALESCE(s.restrict_topics,-9999) AS restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix FROM sections s WHERE s.id=$1"
        );
        Some(
            sqlx::query_as::<_, ViewAllSection>(&sql)
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
    let messages = sqlx::query_as::<_, TopicSummary>(&sql)
        .bind(section.as_ref().map(|s| s.id))
        .fetch_all(&state.pool)
        .await?;
    let uncommitted = messages.len() as i64;

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
    let deleted_topics = sqlx::query_as::<_, DeletedTopicRow>(&sql)
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
        sqlx::query_as::<_, (i32, String, i32, String, i64)>(&sql)
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
    let (add_link, add_link_reason) = match topic_posting_reason(restriction, &user) {
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

    // GroupPermissionService.checkView / drafts: a draft or not-yet-committed
    // premoderated topic is only visible to its author or a moderator. A
    // deleted topic is likewise author/moderator-only - the previous
    // implementation never checked `topic.deleted` at all here, so a
    // deleted topic stayed fully visible to everyone.
    if topic.draft || (topic.section_premoderated && !topic.moderate) || topic.deleted {
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

    let topic_html = markup::render_message_with_markup(&topic.message, Some(&topic.markup), None);

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

    let (page_comments, pages, thread_mode, bHasNextPage): (
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

    let comments: Vec<CommentView> = page_comments
        .into_iter()
        .map(|item| {
            let html = markup::render_message_with_markup(&item.message, Some(&item.markup), None);
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
            CommentView {
                item,
                html,
                reactions_html: reactions.html,
                show_reactions_link: reactions.show_menu_link,
                can_edit,
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
    )
    .await?;
    let images = load_topic_images(&state, topic.id).await?;
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
    let can_comment = !comments_hidden
        && crate::routes::comments::optCommentActorError(
            &state,
            &stPostingResolution.stIdentity.stUser,
            !stPostingResolution.stIdentity.bAuthorized,
            &sRemoteIp,
        )
        .await?
        .is_none()
        && crate::routes::comments::check_comment_posting_allowed(
            &state,
            &stPostingResolution.stIdentity.stUser,
            !stPostingResolution.stIdentity.bAuthorized,
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

    Ok(Html(
        TopicTemplate {
            topic,
            topic_html,
            comments,
            pages,
            thread_mode,
            thread_root,
            show_deleted: want_deleted,
            show_deleted_button: can_view_deleted_comments && !want_deleted,
            filtered_count,
            unfiltered_count,
            csrf_token,
            poll,
            images_html,
            topic_reactions_html: topic_reactions.html,
            topic_show_reactions_link: topic_reactions.show_menu_link,
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
            poll_allowed: group.poll_allowed,
            image_allowed,
            image_required: group.image_required,
            additional_image_rows: if image_allowed && group.section_prefix != "forum" {
                vec![(); 3]
            } else {
                Vec::new()
            },
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
    if form.title.chars().count() > 140 {
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
    let image = image::load_from_memory_with_format(data, format)
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

fn renderSubmittedAddTopicForm(
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
    bPreview: bool,
    bSessionAuthorized: bool,
    bRequireCaptcha: bool,
) -> Result<Response> {
    let (optTopicLimitError, optTopicLimitInfo) = topicLimitNotices(stTopicLimitInfo);
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
            poll_allowed: stGroup.poll_allowed,
            image_allowed: bUploadAllowed,
            image_required: stGroup.image_required,
            additional_image_rows: if bUploadAllowed && stGroup.section_prefix != "forum" {
                vec![(); 3]
            } else {
                Vec::new()
            },
            uploaded_images: stForm.uploaded_images.clone(),
            form_title: stForm.title.clone(),
            form_msg: stForm.msg.clone(),
            form_url: stForm.url.clone().unwrap_or_default(),
            form_linktext: stForm.linktext.clone().unwrap_or_default(),
            form_tags: stForm.tags.clone().unwrap_or_default(),
            preview_html: bPreview
                .then(|| markup::render_message_with_markup(&stForm.msg, Some(sMarkupId), None)),
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
            form.preview.is_some(),
            bSessionAuthorized,
            bRequireCaptcha,
        );
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
            form.preview.is_some(),
            bSessionAuthorized,
            bRequireCaptcha,
        );
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
            true,
            bSessionAuthorized,
            bRequireCaptcha,
        );
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
            false,
            bSessionAuthorized,
            bRequireCaptcha,
        );
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
            false,
            bSessionAuthorized,
            bRequireCaptcha,
        );
    }

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
                sTitle: form.title.trim(),
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

pub async fn edit_topic_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let Some(user) = user else {
        return Ok(crate::routes::auth::login_redirect(&format!(
            "/edit.jsp?msgid={}",
            q.msgid
        )));
    };
    let topic = get_topic(&state, q.msgid).await?;
    let stRules = load_topic_edit_rules(&state, q.msgid).await?;
    check_topic_edit_preconditions(&state, &headers, stPeerAddress, &user, &topic).await?;
    if !b_topic_content_editable(&topic, &stRules, &user)
        && !b_topic_tags_editable(&topic, &stRules, &user)
    {
        return Err(AppError::Forbidden);
    }
    let selected_group = topic.group_id;
    let group = load_topic_form_group(&state, selected_group).await?;
    let (format_mode, format_mode_title) = markup_form_view(&topic.markup);
    let image_allowed = image_upload_allowed(&group, &Some(user));
    let image_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM images WHERE topic=$1 AND NOT deleted")
            .bind(q.msgid)
            .fetch_one(&state.pool)
            .await?;
    let form_msg = if let Some(iRecordId) = q.from_history {
        let cHistoryService = crate::application::edit_history::CEditHistoryService::new(
            crate::infra::postgres::edit_history_repository::CEditHistoryPgRepository::new(
                state.pool.clone(),
            ),
        );
        cHistoryService
            .sRestorableTopicMessage(topic.id, iRecordId)
            .await?
    } else {
        topic.message.clone()
    };
    // PollDao.getPollByTopicId/EditTopicController: pre-fill existing
    // variants (blank if the topic has no poll yet, e.g. a topic moved
    // into the Опросы section after creation) plus a handful of empty
    // slots for adding new ones.
    let poll_row: Option<(i32, bool)> =
        sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1")
            .bind(q.msgid)
            .fetch_optional(&state.pool)
            .await?;
    let (poll_variants, poll_multiselect) = match poll_row {
        Some((poll_id, multiselect)) => {
            let variants = sqlx::query_as::<_, (i32, String)>(
                "SELECT id, label FROM polls_variants WHERE vote=$1 ORDER BY id",
            )
            .bind(poll_id)
            .fetch_all(&state.pool)
            .await?;
            (variants, multiselect)
        }
        None => (Vec::new(), false),
    };
    Ok(Html(
        TopicFormTemplate {
            title: "Редактировать тему".into(),
            form_error: None,
            topic_limit_error: None,
            topic_limit_info: None,
            topic_posting_allowed: true,
            topic_posting_reason: String::new(),
            action: "/edit.jsp".into(),
            topic_id: Some(topic.id),
            csrf_token,
            poll_variants,
            poll_new_rows: if group.poll_allowed {
                vec![String::new(); POLL_NEW_VARIANT_SLOTS]
            } else {
                Vec::new()
            },
            poll_multiselect,
            selected_group,
            is_edit: true,
            links_allowed: group.links_allowed,
            poll_allowed: group.poll_allowed,
            image_allowed,
            image_required: false,
            additional_image_rows: if image_allowed && group.section_prefix != "forum" {
                vec![(); 3usize.saturating_sub(image_count as usize)]
            } else {
                Vec::new()
            },
            uploaded_images: Vec::new(),
            form_title: topic.title.clone(),
            form_msg,
            form_url: topic.url.clone().unwrap_or_default(),
            form_linktext: topic.linktext.clone().unwrap_or_default(),
            form_tags: topic.tags_vec().join(", "),
            preview_html: None,
            noinfo: false,
            add_info_html: None,
            format_mode,
            format_mode_title,
            anonymous_form: false,
            form_nick: String::new(),
            require_captcha: false,
            captcha_site_key: String::new(),
            show_allow_anonymous: false,
            allow_anonymous: true,
        }
        .render()?,
    )
    .into_response())
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

async fn check_topic_edit_preconditions(
    state: &AppState,
    headers: &HeaderMap,
    stPeerAddress: SocketAddr,
    user: &UserSummary,
    topic: &TopicDetail,
) -> Result<()> {
    if topic.deleted {
        return Err(AppError::BadRequest(
            "нельзя править удаленные топики".into(),
        ));
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    if crate::routes::comments::optCommentActorError(state, user, false, &sRemoteIp)
        .await?
        .is_some()
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
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

fn b_topic_tags_editable(topic: &TopicDetail, rules: &TopicEditRules, user: &UserSummary) -> bool {
    user.candel || user.canmod || user.corrector || b_topic_editable_by_author(topic, rules, user)
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
    fn corrector_obeys_no_comments_lock_but_can_still_edit_tags() {
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
        assert!(b_topic_tags_editable(&st_topic, &st_rules, &st_corrector));
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

#[derive(Clone)]
struct TopicPollSnapshot {
    id: i32,
    multiselect: bool,
    variants: Vec<(i32, String)>,
}

async fn load_topic_poll_snapshot(
    state: &AppState,
    topic_id: i32,
) -> Result<Option<TopicPollSnapshot>> {
    let Some((id, multiselect)): Option<(i32, bool)> =
        sqlx::query_as("SELECT id,multiselect FROM polls WHERE topic=$1")
            .bind(topic_id)
            .fetch_optional(&state.pool)
            .await?
    else {
        return Ok(None);
    };
    let variants = sqlx::query_as("SELECT id,label FROM polls_variants WHERE vote=$1 ORDER BY id")
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
    Ok(Some(TopicPollSnapshot {
        id,
        multiselect,
        variants,
    }))
}

fn b_poll_modified(snapshot: Option<&TopicPollSnapshot>, form: &TopicForm) -> bool {
    let bMultiselect = form.multiselect.is_some();
    match snapshot {
        None => form.poll.iter().any(|sLabel| !sLabel.trim().is_empty()),
        Some(stPoll) => {
            if stPoll.multiselect != bMultiselect {
                return true;
            }
            form.variant_id
                .iter()
                .zip(form.poll.iter())
                .any(|(iVariantId, sLabel)| {
                    let sLabel = sLabel.trim();
                    if *iVariantId == 0 {
                        !sLabel.is_empty()
                    } else {
                        stPoll
                            .variants
                            .iter()
                            .find(|(iId, _)| iId == iVariantId)
                            .is_some_and(|(_, sOldLabel)| sOldLabel != sLabel)
                    }
                })
        }
    }
}

fn opt_poll_history_json(
    topic_id: i32,
    snapshot: Option<&TopicPollSnapshot>,
) -> Option<serde_json::Value> {
    snapshot.map(|stPoll| {
        serde_json::json!({
            "id": stPoll.id,
            "topic": topic_id,
            "multiSelect": stPoll.multiselect,
            "variants": stPoll.variants.iter().map(|(iId, sLabel)| {
                serde_json::json!({"id": iId, "label": sLabel})
            }).collect::<Vec<_>>()
        })
    })
}

pub async fn edit_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    request: Request,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let (pairs, uploads) = parse_topic_request(&state, request).await?;
    let mut form = parse_topic_form(&pairs)?;
    if crate::form::get(&pairs, "csrf").map(str::trim) != Some(csrf_token.trim()) {
        return Err(AppError::Forbidden);
    }
    let id = form
        .id
        .ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let meta = load_topic_delete_meta(&state, id).await?;
    let current_topic = get_topic(&state, id).await?;
    let stRules = load_topic_edit_rules(&state, id).await?;
    check_topic_edit_preconditions(&state, &headers, stPeerAddress, &user, &current_topic).await?;
    let group = load_topic_form_group(&state, current_topic.group_id).await?;
    validate_topic_form(&form, group.links_allowed, false)?;
    let bContentEditable = b_topic_content_editable(&current_topic, &stRules, &user);
    let bTagsEditable = b_topic_tags_editable(&current_topic, &stRules, &user);
    if !bContentEditable && !bTagsEditable {
        return Err(AppError::Forbidden);
    }
    let upload_allowed = image_upload_allowed(&group, &Some(user.clone()));
    if (!uploads.is_empty() || !form.uploaded_images.is_empty()) && !upload_allowed {
        return Err(AppError::Forbidden);
    }
    form.uploaded_images = vecReusableTopicPreviews(&state, user.id, &form.uploaded_images);
    let additional_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM images WHERE topic=$1 AND NOT deleted")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if additional_count + uploads.len() as i64 + form.uploaded_images.len() as i64 > 4 {
        return Err(AppError::BadRequest("Слишком много изображений".into()));
    }

    // EditTopicRequestValidator.validateTags: same rule as topic creation.
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(&state, &tags, &user, meta.premoderated).await?;

    let bMessageModified = form.msg != current_topic.message;
    let bTitleModified = form.title.trim() != current_topic.title;
    let optNewUrl = group
        .links_allowed
        .then(|| form.url.as_deref().unwrap_or("").trim().to_owned())
        .filter(|sValue| !sValue.is_empty());
    let optNewLinkText = group
        .links_allowed
        .then(|| form.linktext.as_deref().unwrap_or("").trim().to_owned())
        .filter(|sValue| !sValue.is_empty());
    let bUrlModified =
        optNewUrl.as_deref().unwrap_or("") != current_topic.url.as_deref().unwrap_or("").trim();
    let bLinkTextModified = optNewLinkText.as_deref().unwrap_or("")
        != current_topic.linktext.as_deref().unwrap_or("").trim();
    let vecOldTags = current_topic.tags_vec();
    let mut vecComparableOldTags = vecOldTags
        .iter()
        .map(|sTag| sTag.to_lowercase())
        .collect::<Vec<_>>();
    let mut vecComparableNewTags = tags
        .iter()
        .map(|sTag| sTag.to_lowercase())
        .collect::<Vec<_>>();
    vecComparableOldTags.sort_unstable();
    vecComparableNewTags.sort_unstable();
    let bTagsModified = vecComparableOldTags != vecComparableNewTags;
    let optOldPoll = if meta.poll_allowed {
        load_topic_poll_snapshot(&state, id).await?
    } else {
        None
    };
    let bPollModified = meta.poll_allowed && b_poll_modified(optOldPoll.as_ref(), &form);
    let bContentModified = bMessageModified
        || bTitleModified
        || bUrlModified
        || bLinkTextModified
        || bPollModified
        || !uploads.is_empty()
        || !form.uploaded_images.is_empty();
    if bContentModified && !bContentEditable {
        return Err(AppError::Forbidden);
    }

    if form.preview.is_some() {
        form.uploaded_images
            .extend(vecStageTopicPreviews(&state, user.id, &uploads).await?);
        let poll_variants = form
            .variant_id
            .iter()
            .zip(form.poll.iter())
            .filter(|(id, _)| **id != 0)
            .map(|(id, label)| (*id, label.clone()))
            .collect();
        let poll_new_rows = form
            .variant_id
            .iter()
            .zip(form.poll.iter())
            .filter(|(id, _)| **id == 0)
            .map(|(_, label)| label.clone())
            .collect();
        return Ok(Html(
            TopicFormTemplate {
                title: "Редактирование".into(),
                form_error: None,
                topic_limit_error: None,
                topic_limit_info: None,
                topic_posting_allowed: true,
                topic_posting_reason: String::new(),
                action: "/edit.jsp".into(),
                topic_id: Some(id),
                csrf_token,
                poll_variants,
                poll_new_rows,
                poll_multiselect: form.multiselect.is_some(),
                selected_group: current_topic.group_id,
                is_edit: true,
                links_allowed: group.links_allowed,
                poll_allowed: group.poll_allowed,
                image_allowed: upload_allowed,
                image_required: false,
                additional_image_rows: if upload_allowed && group.section_prefix != "forum" {
                    vec![(); 3usize.saturating_sub(additional_count as usize)]
                } else {
                    Vec::new()
                },
                uploaded_images: form.uploaded_images.clone(),
                form_title: form.title.clone(),
                form_msg: form.msg.clone(),
                form_url: form.url.clone().unwrap_or_default(),
                form_linktext: form.linktext.clone().unwrap_or_default(),
                form_tags: form.tags.clone().unwrap_or_default(),
                preview_html: Some(markup::render_message_with_markup(
                    &form.msg,
                    Some(&current_topic.markup),
                    None,
                )),
                noinfo: false,
                add_info_html: None,
                format_mode: markup_form_view(&current_topic.markup).0,
                format_mode_title: markup_form_view(&current_topic.markup).1,
                anonymous_form: false,
                form_nick: String::new(),
                require_captcha: false,
                captcha_site_key: String::new(),
                show_allow_anonymous: false,
                allow_anonymous: true,
            }
            .render()?,
        )
        .into_response());
    }

    let bModified = bContentModified || bTagsModified;
    if !bModified {
        return Err(AppError::BadRequest("Нет изменений".into()));
    }
    let vecOldImageIds: Vec<i32> = if uploads.is_empty() && form.uploaded_images.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT id FROM images WHERE topic=$1 AND NOT deleted ORDER BY id")
            .bind(id)
            .fetch_all(&state.pool)
            .await?
    };

    let mut stUploadRollback = StTopicUploadRollback::stNew();
    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    if bMessageModified {
        service.vUpdateTopicMessage(&mut tx, id, &form.msg).await?;
    }
    if bTitleModified || bUrlModified || bLinkTextModified {
        service
            .vUpdateTopicHeader(
                &mut tx,
                StEditTopic {
                    iMsgId: id,
                    sTitle: form.title.trim(),
                    optUrl: optNewUrl,
                    optLinkText: optNewLinkText,
                },
            )
            .await?;
    }
    if bTagsModified {
        service
            .vReplaceTags(&mut tx, id, form.tags.as_deref())
            .await?;
    }
    if bPollModified {
        save_poll(
            &mut tx,
            id,
            form.multiselect.is_some(),
            &form.variant_id,
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
    sqlx::query(
        r#"INSERT INTO edit_info(
             msgid,editor,oldmessage,oldtitle,oldtags,oldlinktext,oldurl,
             object_type,oldpoll,oldaddimages
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'TOPIC'::edit_event_type,$8,$9)"#,
    )
    .bind(id)
    .bind(user.id)
    .bind(bMessageModified.then_some(current_topic.message.as_str()))
    .bind(bTitleModified.then_some(current_topic.title.as_str()))
    .bind(bTagsModified.then(|| vecOldTags.join(", ")))
    .bind(
        bLinkTextModified
            .then_some(current_topic.linktext.as_deref())
            .flatten(),
    )
    .bind(
        bUrlModified
            .then_some(current_topic.url.as_deref())
            .flatten(),
    )
    .bind(
        bPollModified
            .then(|| opt_poll_history_json(id, optOldPoll.as_ref()))
            .flatten()
            .map(sqlx::types::Json),
    )
    .bind((!uploads.is_empty() || !form.uploaded_images.is_empty()).then_some(vecOldImageIds))
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let vecNotified = if !current_topic.draft && !stRules.expired {
        notify_topic_users_tx(
            &mut tx,
            id,
            current_topic.author_id,
            &form.msg,
            !current_topic.section_premoderated,
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
    state.realtime.vNotifyEvents(vecNotified.iter().copied());
    crate::search_index::index_topic(&state, id, false).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")).into_response())
}

#[derive(Deserialize)]
pub struct TopicActionForm {
    pub msgid: i32,
    pub resolve: Option<String>,
    pub reason: Option<String>,
    pub bonus: Option<i32>,
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
                && dtNow <= meta.postdate + chrono::Duration::hours(TOPIC_DELETE_WINDOW_HOURS)));
    if user.candel || bDeletableByAuthor {
        true
    } else if user.canmod {
        !meta.premoderated || !meta.commited || dtNow <= meta.postdate + chrono::Duration::days(30)
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
    poll_allowed: bool,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StTopicActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

type TyTopicDeleteRow = (
    i32,
    bool,
    chrono::DateTime<chrono::Utc>,
    bool,
    bool,
    bool,
    i64,
    bool,
);

async fn b_user_slow_mode_restricted(state: &AppState, user: &UserSummary) -> Result<bool> {
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
                  t.moderate, t.stat1::bigint, s.vote
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
        poll_allowed: row.7,
    })
}

pub async fn delete_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<TopicActionForm>,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if form.bonus.is_some_and(|iBonus| !(0..=20).contains(&iBonus)) {
        return Err(AppError::BadRequest("неправильный размер штрафа".into()));
    }
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if meta.deleted {
        return Err(AppError::BadRequest("сообщение уже удалено".into()));
    }

    if !b_topic_deletable(&meta, &user, chrono::Utc::now()) {
        return Err(AppError::Forbidden);
    }

    let mut bonus = if user.canmod && user.id != meta.author_id && !meta.draft {
        crate::routes::comments::iDeleteScoreDelta(form.bonus.unwrap_or(0))
    } else {
        0
    };
    if bonus != 0 && meta.author_id != 2 {
        let bAuthorFrozen: bool = sqlx::query_scalar(
            "SELECT COALESCE(frozen_until > CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
        )
        .bind(meta.author_id)
        .fetch_one(&state.pool)
        .await?;
        if bAuthorFrozen {
            bonus = 0;
        }
    }
    let reason = form.reason.clone().unwrap_or_default();

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE topics SET deleted=true,sticky=false WHERE id=$1 AND NOT deleted")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&mut *tx).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score+$2,0) WHERE id=$1")
            .bind(meta.author_id)
            .bind(bonus)
            .execute(&mut *tx)
            .await?;
    }
    vDeleteTopicEventsTx(&mut tx, form.msgid).await?;
    crate::routes::comments::vNotifyDeletedTx(
        &mut tx,
        meta.author_id,
        user.id,
        Some(form.msgid),
        None,
        &reason,
    )
    .await?;
    tx.commit().await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Html(
        StTopicActionDoneTemplate {
            message: "Сообщение удалено".into(),
            big_message: None,
            link: None,
        }
        .render()?,
    ))
}

/// UserEventService.processTopicDeleted, kept in the write transaction just
/// like DeleteService.deleteTopic in the original application.
async fn vDeleteTopicEventsTx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    iTopicId: i32,
) -> Result<()> {
    let vecAffectedUsers: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT userid FROM user_events
           WHERE message_id=$1
             AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(iTopicId)
    .fetch_all(&mut **tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events
           WHERE message_id=$1
             AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(iTopicId)
    .execute(&mut **tx)
    .await?;
    if !vecAffectedUsers.is_empty() {
        sqlx::query(
            r#"UPDATE users SET unread_events=(
                   SELECT count(*) FROM user_events e
                   WHERE e.unread AND e.userid=users.id
               ) WHERE id = ANY($1)"#,
        )
        .bind(&vecAffectedUsers)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn undelete_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<TopicActionForm>,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if !user.canmod {
        return Err(AppError::Forbidden);
    }
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if !bTopicUndeletable(&state, &meta, &user, form.msgid).await? {
        return Err(AppError::Forbidden);
    }
    let mut tx = state.pool.begin().await?;
    let optBonus: Option<i32> =
        sqlx::query_scalar("SELECT bonus FROM del_info WHERE msgid=$1 FOR UPDATE")
            .bind(form.msgid)
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    if let Some(iBonus) = optBonus.filter(|iValue| *iValue != 0) {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1")
            .bind(meta.author_id)
            .bind(iBonus)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE topics SET deleted=false WHERE id=$1")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    let topic = get_topic(&state, form.msgid).await?;
    Ok(Html(
        StTopicActionDoneTemplate {
            message: "Сообщение восстановлено".into(),
            big_message: None,
            link: Some(topic.topic_url()),
        }
        .render()?,
    ))
}

pub async fn resolve_topic_get(
    State(state): State<AppState>,
    Query(form): Query<TopicActionForm>,
    CurrentUser(user): CurrentUser,
) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

pub async fn resolve_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<TopicActionForm>,
) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

async fn do_resolve_topic(
    state: &AppState,
    user: Option<crate::models::UserSummary>,
    form: TopicActionForm,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let Some((author_id, group_resolvable)) =
        topic_service(state).optResolveMeta(form.msgid).await?
    else {
        return Err(AppError::NotFound);
    };
    if !group_resolvable {
        return Err(AppError::Forbidden);
    }
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }
    let resolved = form.resolve.as_deref().map(|value| value == "yes");
    topic_service(state)
        .vSetResolved(form.msgid, resolved)
        .await?;
    Ok(Redirect::to(&format!(
        "/jump-message.jsp?msgid={}",
        form.msgid
    )))
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
    bIncludeTagEvents: bool,
) -> Result<Vec<i32>> {
    let mentioned_nicks = markup::extract_mentions(message);
    let mut notified: Vec<i32> = if mentioned_nicks.is_empty() {
        vec![]
    } else {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick) = ANY($1) AND u.id <> $2
                 AND NOT EXISTS (
                     SELECT 1 FROM topic_users_notified tun
                     WHERE tun.topic=$3 AND tun.userid=u.id
                 )
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(mentioned_nicks.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>())
        .bind(author_id)
        .bind(topic_id)
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

pub async fn delete_topic_form(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let stMeta = load_topic_delete_meta(&state, q.msgid).await?;
    if stMeta.deleted {
        return Err(AppError::BadRequest("Сообщение уже удалено".into()));
    }
    if !b_topic_deletable(&stMeta, &user, chrono::Utc::now()) {
        return Err(AppError::Forbidden);
    }
    let bExpired = crate::routes::comments::is_topic_expired(&state, q.msgid).await?;
    let optAuthorScore = if user.canmod && !stMeta.premoderated && !stMeta.draft && !bExpired {
        Some(
            sqlx::query_scalar::<_, i32>("SELECT COALESCE(score,0) FROM users WHERE id=$1")
                .bind(stMeta.author_id)
                .fetch_one(&state.pool)
                .await?,
        )
    } else {
        None
    };
    let optBonus = optAuthorScore
        .map(|iScore| {
            format!(
                r#"<div class="control-group"><label for="bonus-input">Штраф<br>score автора: {}</label><div class="controls"><input id="bonus-input" type="number" name="bonus" value="7" min="0" max="20"><span class="help-inline">(от 0 до 20)</span></div></div>"#,
                iScore
            )
        })
        .unwrap_or_default();
    Ok(Html(format!(
        r#"
<h1>Удаление сообщения</h1>
<form method="post" action="/delete.jsp" class="form-horizontal">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <div class="control-group"><label class="control-label" for="reason-input">Причина удаления</label><div class="controls"><input id="reason-input" type="text" name="reason"></div></div>
  {optBonus}
  <input type="hidden" name="msgid" value="{0}">
  <div class="control-group"><div class="controls"><button type="submit" class="btn btn-danger">Удалить</button></div></div>
</form>
"#,
        q.msgid
    )))
}

pub async fn undelete_topic_form(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let stMeta = load_topic_delete_meta(&state, q.msgid).await?;
    if !bTopicUndeletable(&state, &stMeta, &user, q.msgid).await? {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Восстановить тему #{}</h1>
<form method="post" action="/undelete">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#,
        q.msgid, q.msgid
    )))
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
    if user.corrector && !user.canmod && user.id == topic_author_id {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub async fn commit_topic_form(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1")
        .bind(q.msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
    Ok(Html(format!(
        r#"
<h1>Подтвердить тему #{}</h1>
<form method="post" action="/commit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Подтвердить</button>
</form>
"#,
        q.msgid, q.msgid
    )))
}

pub async fn commit_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<TopicActionForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1")
        .bind(form.msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
    topic_service(&state)
        .vCommitTopic(form.msgid, user.id)
        .await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!(
        "/jump-message.jsp?msgid={}",
        form.msgid
    )))
}

pub async fn uncommit_form(
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Отменить подтверждение темы #{}</h1>
<form method="post" action="/uncommit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Отменить подтверждение</button>
</form>
"#,
        q.msgid, q.msgid
    )))
}

pub async fn uncommit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<TopicActionForm>,
) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    topic_service(&state).vUncommitTopic(form.msgid).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!(
        "/jump-message.jsp?msgid={}",
        form.msgid
    )))
}

#[derive(Deserialize)]
pub struct MoveTopicForm {
    pub msgid: i32,
    pub moveto: i32,
}

pub async fn move_topic_form(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    let mut options = String::new();
    for g in groups {
        let selected = if g.id == topic.group_id {
            " selected"
        } else {
            ""
        };
        options.push_str(&format!(
            "<option value=\"{}\"{}>{} / {}</option>",
            g.id,
            selected,
            html_escape::encode_text(&g.section_name),
            html_escape::encode_text(&g.title)
        ));
    }
    Ok(Html(format!(
        r#"
<h1>Переместить тему #{}</h1>
<form method="post" action="/mt.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <select name="moveto">{}</select>
  <button type="submit">Переместить</button>
</form>
"#,
        q.msgid, q.msgid, options
    )))
}

pub async fn move_topic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<MoveTopicForm>,
) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    topic_service(&state)
        .vMoveTopic(form.msgid, form.moveto)
        .await?;
    Ok(Redirect::to(&format!(
        "/jump-message.jsp?msgid={}",
        form.msgid
    )))
}

pub async fn premoderated_move_form(
    State(state): State<AppState>,
    Query(q): Query<ViewMessageQuery>,
    user: CurrentUser,
    csrf: crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    move_topic_form(State(state), Query(q), user, csrf).await
}
