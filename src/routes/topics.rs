use crate::{auth::CurrentUser, application::topic::CTopicService, domain::topic::repository::{StEditTopic, StNewTopic}, error::{AppError, Result}, infra::postgres::topic_repository::CTopicPgRepository, markup, models::{CommentItem, Group, PagerQuery, TagItem, TopicDetail, TopicSummary, UserSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{FromRequest, Multipart, Path, Query, Request, State}, http::{header::CONTENT_TYPE, Uri}, response::{Html, IntoResponse, Redirect, Response}, Form};
use serde::Deserialize;

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
}

#[derive(Debug, Clone)]
struct CommentView {
    item: CommentItem,
    html: String,
    reactions_html: String,
    show_reactions_link: bool,
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

fn upload_image_url(path: &str) -> String {
    if path.starts_with('/') || path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else if path.starts_with("images/") {
        format!("/{path}")
    } else if let Some(path) = path.strip_prefix("gallery/") {
        format!("/gallery-uploads/{path}")
    } else {
        format!("/gallery-uploads/{path}")
    }
}

fn scaled_dimensions(width: i32, height: i32, max_side: i32) -> (i32, i32) {
    if width <= 0 || height <= 0 || width.max(height) <= max_side {
        return (width.max(1), height.max(1));
    }
    if width >= height {
        (max_side, (i64::from(height) * i64::from(max_side) / i64::from(width)) as i32)
    } else {
        ((i64::from(width) * i64::from(max_side) / i64::from(height)) as i32, max_side)
    }
}

pub(crate) async fn load_topic_images(state: &AppState, topic_id: i32) -> Result<Vec<TopicImageView>> {
    let rows: Vec<(i32, Option<String>, Option<String>, Option<i32>, Option<i32>, Option<String>)> = sqlx::query_as(
        "SELECT id, medium, original, width, height, extension FROM images WHERE topic=$1 AND NOT deleted ORDER BY (COALESCE(primary_image,false) OR COALESCE(main,false)) DESC, id",
    )
    .bind(topic_id)
    .fetch_all(&state.pool)
    .await?;
    let mut prepared = Vec::with_capacity(rows.len());
    for (id, medium, original, stored_width, stored_height, extension) in rows {
        let original = original.or_else(|| extension.as_ref().map(|extension| format!("images/{id}/original.{extension}")));
        let Some(original) = original else { continue; };
        let dimensions = if stored_width.is_none() || stored_height.is_none() {
            let path = format!("{}/{}", state.config.upload_dir, original.trim_start_matches('/'));
            tokio::task::spawn_blocking(move || image::image_dimensions(path).ok()).await.unwrap_or(None)
        } else {
            None
        };
        let width = stored_width.or_else(|| dimensions.map(|value| value.0 as i32)).unwrap_or(1000).max(1);
        let height = stored_height.or_else(|| dimensions.map(|value| value.1 as i32)).unwrap_or(1000).max(1);
        let medium = medium.unwrap_or_else(|| format!("images/{id}/1000px.jpg"));
        let (medium_width, medium_height) = scaled_dimensions(width, height, 1000);
        let srcset = if original.starts_with("images/") {
            let mut values = [500, 1000, 1500, 2000].into_iter()
                .filter(|size| width > 2000 || *size < width)
                .map(|size| (format!("/images/{id}/{size}px.jpg"), size))
                .collect::<Vec<_>>();
            if width <= 2000 {
                values.push((upload_image_url(&original), width));
            }
            values
        } else {
            vec![(upload_image_url(&medium), medium_width), (upload_image_url(&original), width)]
        };
        prepared.push(TopicImageView {
            medium_url: upload_image_url(&medium),
            original_url: upload_image_url(&original),
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
    image.srcset.iter().map(|(url, width)| format!("{url} {width}w")).collect::<Vec<_>>().join(", ")
}

pub(crate) fn topic_image_srcset(image: &TopicImageView) -> String {
    image_srcset(image)
}

fn render_single_image(image: &TopicImageView, title: &str, imagepost: bool, news: bool) -> String {
    let height_limit = if news { "70vh" } else { "90vh" };
    let sizes = if news { "(min-width: 47em) 40vw, 100vw" } else { "(min-width: 70em) 80vw, 100vw" };
    let max_width = image.width.min(2000);
    let padding = 100.0 * f64::from(image.medium_height) / f64::from(image.medium_width);
    let title = html_escape::encode_double_quoted_attribute(title);
    let src = html_escape::encode_double_quoted_attribute(&image.medium_url);
    let original = html_escape::encode_double_quoted_attribute(&image.original_url);
    let srcset_value = image_srcset(image);
    let srcset = html_escape::encode_double_quoted_attribute(&srcset_value);
    let linked = imagepost || image.width >= 1920 || image.height >= 1080;
    let open_link = if linked { format!(r#"<a href="{original}" itemprop="contentURL">"#) } else { String::new() };
    let close_link = if linked { "</a>" } else { "" };
    format!(r#"<div class="medium-image-container" style="max-width: {max_width}px; max-height: {height_limit}; width: min(var(--image-width), calc({height_limit} * {mw} / {mh}))">
<figure class="medium-image" style="position: relative; padding-bottom: {padding}%; padding-bottom: min({padding}%, {height_limit}); margin: 0" itemprop="associatedMedia" itemscope itemtype="http://schema.org/ImageObject">
{open_link}<img itemprop="thumbnail" class="medium-image" src="{src}" alt="{title}" srcset="{srcset}" sizes="{sizes}" style="position: absolute; max-height: {height_limit}" width="{mw}" height="{mh}">{close_link}
<meta itemprop="caption" content="{title}">
</figure></div>"#, mw=image.medium_width, mh=image.medium_height)
}

fn render_image_slider(images: &[TopicImageView], title: &str, news: bool) -> String {
    let main = &images[0];
    let height_limit = if news { "70vh" } else { "90vh" };
    let sizes = if news { "(min-width: 47em) 40vw, 100vw" } else { "(min-width: 70em) 80vw, 100vw" };
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
        indicators.push_str(&format!(r#"<a href="{original}"{}></a>"#, if index == 0 { " class=\"active\"" } else { "" }));
    }
    format!(r#"<div class="slider-parent" style="width: min(var(--image-width), calc({height_limit} * {mw} / {mh}))">
<div class="swiffy-slider slider-indicators-round {classes} slider-item-ratio slider-item-ratio-contain" style="--swiffy-slider-item-ratio: {fw}/{fh}">
<div class="slider-container">{items}</div>
<button type="button" class="slider-nav" aria-label="Предыдущее изображение"></button>
<button type="button" class="slider-nav slider-nav-next" aria-label="Следующее изображение"></button>
<div class="slider-indicators">{indicators}</div>
</div></div>"#, mw=main.medium_width, mh=main.medium_height, fw=main.width, fh=main.height)
}

fn render_topic_images(images: &[TopicImageView], title: &str, imagepost: bool, news: bool) -> String {
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
            medium_url: format!("/gallery-uploads/{id}/medium.jpg"),
            original_url: format!("/gallery-uploads/{id}/original.jpg"),
            width: 1920,
            height: 1080,
            medium_width: 800,
            medium_height: 450,
            srcset: vec![
                (format!("/gallery-uploads/{id}/thumbnail.jpg"), 200),
                (format!("/gallery-uploads/{id}/medium.jpg"), 800),
                (format!("/gallery-uploads/{id}/original.jpg"), 1920),
            ],
        }
    }

    #[test]
    fn one_image_uses_the_original_responsive_container() {
        let html = render_topic_images(&[image(1)], "Заголовок", false, true);
        assert!(html.contains("medium-image-container"));
        assert!(html.contains("(min-width: 47em) 40vw, 100vw"));
        assert!(html.contains("thumbnail.jpg 200w"));
        assert!(html.contains("max-height: 70vh"));
    }

    #[test]
    fn several_images_use_the_original_slider_dom() {
        let html = render_topic_images(&[image(1), image(2)], "Заголовок", false, false);
        assert!(html.contains("swiffy-slider"));
        assert!(html.contains("slider-nav-next"));
        assert!(html.contains("slider-indicators"));
        assert!(html.contains("/gallery-uploads/1/medium.jpg"));
        assert!(html.contains("/gallery-uploads/2/medium.jpg"));
    }
}

pub(crate) async fn prepare_news_topics(state: &AppState, topics: Vec<TopicSummary>, show_group: bool) -> Result<Vec<NewsTopicView>> {
    let mut prepared = Vec::with_capacity(topics.len());
    for topic in topics {
        let row: Option<(String, Option<bool>, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT m.message, m.bbcode, m.markup, t.linktext, g.image FROM msgbase m JOIN topics t ON t.id=m.id JOIN groups g ON g.id=t.groupid WHERE m.id=$1",
        )
        .bind(topic.id)
        .fetch_optional(&state.pool)
        .await?;
        let (message, bbcode, message_markup, linktext, group_image) = row
            .unwrap_or_else(|| (String::new(), Some(true), "BBCODE_TEX".into(), None, None));
        let images = load_topic_images(state, topic.id).await?;
        let images_html = render_topic_images(&images, &topic.title, topic.section_prefix == "gallery", true);
        let group_image_url = group_image.map(|path| {
            if path.starts_with('/') { format!("/tango{path}") } else { format!("/tango/{path}") }
        });
        prepared.push(NewsTopicView {
            topic_html: markup::render_message_with_markup(&message, Some(&message_markup), bbcode),
            images_html,
            group_image_url,
            linktext: linktext.filter(|value| !value.is_empty()).unwrap_or_else(|| "Подробности".to_string()),
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
fn reactions_allow_interact(current_user: &Option<UserSummary>, frozen: bool, topic_expired: bool, target_author_id: i32, target_deleted: bool, comments_hidden: bool) -> bool {
    match current_user {
        Some(u) => u.id != target_author_id && !frozen && !topic_expired && !target_deleted && !comments_hidden,
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
    let anon_class = if is_anonymous { " reaction-anonymous" } else { "" };
    let disabled = if allow_interact { "" } else { " disabled" };

    struct Btn { emoji: String, count: i64, clicked: bool, tooltip: String }
    let mut buttons = Vec::new();
    for (emoji, description) in crate::routes::api::REACTIONS {
        let mut users: Vec<&(String, i32, String, i32)> = reaction_users.iter().filter(|(r, ..)| r == emoji).collect();
        users.sort_by(|a, b| b.3.cmp(&a.3));
        let count = users.len() as i64;
        let clicked = current_user_id.map(|uid| users.iter().any(|(_, u, ..)| *u == uid)).unwrap_or(false);
        let top: Vec<&str> = users.iter().take(3).map(|(_, _, nick, _)| nick.as_str()).collect();
        let more = if users.len() > 3 { "..." } else { "" };
        let tooltip = format!("Реакция \"{description}\": {}{more}", top.join(" "));
        buttons.push(Btn { emoji: emoji.to_string(), count, clicked, tooltip });
    }

    // PreparedReactions uses a TreeMap, so preserve the original UTF-16
    // string order rather than the declaration order of REACTIONS.
    buttons.sort_by_key(|button| button.emoji.encode_utf16().collect::<Vec<_>>());

    let has_reactions = buttons.iter().any(|button| button.count > 0);
    let outer_class = if has_reactions { "reactions" } else { "reactions zero-reactions" };

    let mut html = format!(
        "<div class=\"{outer_class}\"><form class=\"reactions-form\" action=\"/reactions\" method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"topic\" value=\"{topic_id}\">",
        html_escape::encode_double_quoted_attribute(csrf_token),
    );
    if let Some(cid) = comment_id {
        html.push_str(&format!("<input type=\"hidden\" name=\"comment\" value=\"{cid}\">"));
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
        let comment_query = comment_id.map(|id| format!("&comment={id}")).unwrap_or_default();
        html.push_str(&format!(
            "<a class=\"reaction reaction-show-list\" href=\"/reactions?topic={topic_id}{comment_query}\">?</a>",
        ));
    }
    if allow_interact && buttons.iter().any(|button| button.count == 0) {
        if has_reactions {
            let comment_query = comment_id.map(|id| format!("&comment={id}")).unwrap_or_default();
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
        assert!(widget.html.starts_with("<div class=\"reactions zero-reactions\">"));
        assert!(!widget.html.contains("class=\"reaction reaction-show\""));
        assert!(widget.html.contains("name=\"reaction\""));
        assert!(widget.html.contains("<span class=\"reaction-count\">0</span>"));
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
        assert!(widget.html.contains("href=\"/reactions?topic=42&comment=9\">?</a>"));
        assert!(widget.html.contains("class=\"reaction reaction-show\""));
        assert!(widget.html.contains("<span class=\"zero-reactions\">"));
        assert!(widget.html.find("🎉").unwrap() < widget.html.find("👍").unwrap());
    }

    #[test]
    fn anonymous_empty_widget_has_no_reveal_link_or_buttons() {
        let widget = render_reactions_widget(42, None, &[], None, false, "token");

        assert!(!widget.show_menu_link);
        assert!(widget.html.starts_with("<div class=\"reactions zero-reactions\">"));
        assert!(!widget.html.contains("name=\"reaction\""));
    }
}

/// All reactions for the topic in one query (topic-level rows have
/// `comment_id IS NULL`), so per-comment widgets don't each hit the DB.
/// The JSON maps on topics/comments are the authoritative state in Java;
/// reactions_log is only an audit/date source and can be incomplete in an
/// imported database.
async fn load_all_reactions(state: &AppState, topic_id: i32) -> Result<Vec<(Option<i32>, String, i32, String, i32)>> {
    Ok(sqlx::query_as(
        r#"SELECT NULL::integer AS comment_id, item.value AS reaction,
                  item.key::integer AS origin_user, u.nick, COALESCE(u.score,0)
           FROM topics t
           CROSS JOIN LATERAL jsonb_each_text(COALESCE(t.reactions,'{}'::jsonb)) item
           JOIN users u ON u.id=item.key::integer
           WHERE t.id=$1 AND item.key ~ '^[0-9]+$'
           UNION ALL
           SELECT c.id AS comment_id, item.value AS reaction,
                  item.key::integer AS origin_user, u.nick, COALESCE(u.score,0)
           FROM comments c
           CROSS JOIN LATERAL jsonb_each_text(COALESCE(c.reactions,'{}'::jsonb)) item
           JOIN users u ON u.id=item.key::integer
           WHERE c.topic=$1 AND item.key ~ '^[0-9]+$'"#,
    )
    .bind(topic_id)
    .fetch_all(&state.pool)
    .await?)
}

async fn load_poll_view(state: &AppState, topic_id: i32, deleted: bool, pending: bool, expired: bool, results_requested: bool, current_user: &Option<UserSummary>) -> Result<Option<PollView>> {
    let Some((poll_id, multiselect)): Option<(i32, bool)> = sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1").bind(topic_id).fetch_optional(&state.pool).await? else {
        return Ok(None);
    };
    let current_user_id = current_user.as_ref().map(|user| user.id).unwrap_or(0);
    let mut rows: Vec<(i32, String, i32, bool)> = sqlx::query_as(
        "SELECT v.id,v.label,v.votes,EXISTS(SELECT 1 FROM vote_users u WHERE u.vote=v.vote AND u.variant_id=v.id AND u.userid=$2) FROM polls_variants v WHERE v.vote=$1 ORDER BY v.id",
    ).bind(poll_id).bind(current_user_id).fetch_all(&state.pool).await?;
    let total_votes: i32 = rows.iter().map(|(_, _, votes, _)| *votes).sum();
    let total_people: i64 = sqlx::query_scalar("SELECT count(DISTINCT userid) FROM vote_users WHERE vote=$1").bind(poll_id).fetch_one(&state.pool).await?;
    let user_voted = rows.iter().any(|row| row.3);
    let show_results = !pending && (results_requested || user_voted || expired);
    if show_results { rows.sort_by_key(|(id, _, votes, _)| (std::cmp::Reverse(*votes), *id)); }
    let divisor = if total_people > 0 { total_people as i32 } else { total_votes };
    let max_votes = rows.iter().map(|row| row.2).max().unwrap_or(0);
    let variants = rows.into_iter().map(|(id, label, votes, selected)| PollVariantView {
        id, label, votes,
        pct: if divisor > 0 { ((100.0 * f64::from(votes) / f64::from(divisor)).round()) as i32 } else { 0 },
        progress_pct: if max_votes > 0 { ((320 * votes / max_votes) / 16) * 16 * 100 / 320 } else { 0 },
        user_voted: selected,
    }).collect();
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

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
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
}

async fn user_format_mode(state: &AppState, user_id: i32) -> Result<(String, String, String)> {
    let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
        .bind(user_id).fetch_optional(&state.pool).await?.flatten();
    let mode = crate::profile::ProfileSettings::from_hstore_text(settings_text).format_mode;
    let title = crate::profile::FORMAT_MODES.iter().find(|(id, _, _)| *id == mode)
        .map(|(_, title, _)| *title).unwrap_or("Markdown").to_string();
    let markup = match mode.as_str() {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "plain" => "PLAIN",
        _ => "BBCODE_TEX",
    };
    Ok((mode, title, markup.to_string()))
}

fn markup_form_view(markup: &str, bbcode: Option<bool>) -> (String, String) {
    match markup {
        "MARKDOWN" => ("markdown".into(), "Markdown".into()),
        "BBCODE_ULB" => ("ntobr".into(), "User line break".into()),
        "PLAIN" => ("plain".into(), "HTML".into()),
        "BBCODE_TEX" | "LORCODE" => ("lorcode".into(), "LORCODE".into()),
        _ if bbcode == Some(false) => ("markdown".into(), "Markdown".into()),
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
}

/// `axum::Form` can't deserialize the repeated `poll`/`variant_id` keys into
/// `Vec` fields (see `crate::form`), so this form is parsed from the raw
/// body by hand instead.
fn parse_indexed_field(pairs: &[(String, String)], prefix: &str) -> Vec<(i32, String)> {
    let start = format!("{prefix}[");
    let mut values: Vec<(i32, String)> = pairs.iter().filter_map(|(key, value)| {
        key.strip_prefix(&start)?.strip_suffix(']')?.parse().ok().map(|index| (index, value.clone()))
    }).collect();
    values.sort_by_key(|(index, _)| *index);
    values
}

fn parse_topic_form(pairs: &[(String, String)]) -> Result<TopicForm> {
    use crate::form::{get, get_all};
    let indexed_poll = parse_indexed_field(pairs, "poll");
    let new_poll = parse_indexed_field(pairs, "newPoll");
    let (poll, variant_id) = if !indexed_poll.is_empty() || !new_poll.is_empty() {
        let mut ids = indexed_poll.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let mut labels = indexed_poll.into_iter().map(|(_, label)| label).collect::<Vec<_>>();
        ids.extend(std::iter::repeat_n(0, new_poll.len()));
        labels.extend(new_poll.into_iter().map(|(_, label)| label));
        (labels, ids)
    } else {
        // Accept the first Rust port's flattened fields as a compatibility
        // fallback, while every generated form uses Java's indexed names.
        (
            get_all(pairs, "poll").into_iter().map(str::to_string).collect(),
            get_all(pairs, "variant_id").into_iter().filter_map(|s| s.parse().ok()).collect(),
        )
    };
    Ok(TopicForm {
        id: get(pairs, "msgid").or_else(|| get(pairs, "id")).and_then(|v| v.parse().ok()),
        group: get(pairs, "group").and_then(|v| v.parse().ok()).unwrap_or(0),
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
        multiselect: get(pairs, "multiselect").or_else(|| get(pairs, "multiSelect")).map(str::to_string),
    })
}

#[cfg(test)]
mod topic_form_contract_tests {
    use super::*;

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect()
    }

    #[test]
    fn parses_java_add_topic_poll_contract() {
        let form = parse_topic_form(&pairs(&[
            ("group", "19387"), ("title", "Опрос"), ("msg", "Текст"), ("tags", "lor"),
            ("poll[1]", "Второй"), ("poll[0]", "Первый"), ("multiSelect", "true"),
        ])).unwrap();
        assert_eq!(form.group, 19387);
        assert_eq!(form.poll, ["Первый", "Второй"]);
        assert_eq!(form.variant_id, [0, 1]);
        assert!(form.multiselect.is_some());
    }

    #[test]
    fn parses_java_edit_topic_poll_contract_without_group() {
        let form = parse_topic_form(&pairs(&[
            ("msgid", "42"), ("title", "Опрос"), ("msg", "Текст"), ("tags", "lor"),
            ("poll[17]", "Существующий"), ("newPoll[0]", "Новый"), ("multiselect", "on"),
        ])).unwrap();
        assert_eq!(form.id, Some(42));
        assert_eq!(form.group, 0);
        assert_eq!(form.poll, ["Существующий", "Новый"]);
        assert_eq!(form.variant_id, [17, 0]);
        assert!(form.multiselect.is_some());
    }

    #[test]
    fn accepts_legacy_flattened_rust_fields_during_transition() {
        let form = parse_topic_form(&pairs(&[
            ("group", "8"), ("title", "Опрос"), ("msg", "Текст"), ("tags", "lor"),
            ("variant_id", "12"), ("poll", "Да"), ("variant_id", "0"), ("poll", "Нет"),
        ])).unwrap();
        assert_eq!(form.poll, ["Да", "Нет"]);
        assert_eq!(form.variant_id, [12, 0]);
    }
}

pub async fn index(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let _ = q;
    let show_gallery_on_main = match &current_user {
        Some(user) => {
            let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(user.id).fetch_optional(&state.pool).await?.flatten();
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
            right.sticky.cmp(&left.sticky).then_with(|| {
                right_date.cmp(left_date)
            })
        });
        topics.truncate(30);
        topics
    } else {
        list_topics(&state, Some("news"), None, 0, 30).await?
    };
    let news = prepare_news_topics(&state, all_topics.iter().take(10).cloned().collect(), true).await?;
    let brief = all_topics.iter().skip(10).cloned().collect();
    let add_restriction: i32 = if show_gallery_on_main {
        sqlx::query_scalar("SELECT min(restrict_score) FROM sections")
            .fetch_one(&state.pool)
            .await?
    } else {
        sqlx::query_scalar("SELECT restrict_score FROM sections WHERE id=1")
            .fetch_one(&state.pool)
            .await?
    };
    let add_reason = posting_reason_for_port(&state, add_restriction, &current_user).await?;
    let mut uncommitted = sqlx::query_as::<_, (i32, String, i64)>(
        "SELECT s.id,s.name,count(t.id) FROM sections s JOIN groups g ON g.section=s.id JOIN topics t ON t.groupid=g.id WHERE t.moderate AND NOT t.deleted AND NOT t.draft GROUP BY s.id,s.name HAVING count(t.id)>0 ORDER BY s.id",
    ).fetch_all(&state.pool).await?;
    let can_review_all_sections = current_user.as_ref().is_some_and(|user| user.canmod || user.corrector);
    if !show_gallery_on_main && !can_review_all_sections {
        uncommitted.retain(|(section_id, _, _)| *section_id == 1);
    }
    let (drafts_count, favorite_present, user_status) = match &current_user {
        Some(user) => {
            let drafts: i64 = sqlx::query_scalar("SELECT count(*) FROM topics WHERE userid=$1 AND draft AND NOT deleted").bind(user.id).fetch_one(&state.pool).await?;
            let favorites: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memories WHERE userid=$1 AND watch=false)").bind(user.id).fetch_one(&state.pool).await?;
            let status = if user.score.unwrap_or(0) >= 100 { "активный пользователь" } else { "новый пользователь" };
            (drafts, favorites, status.to_string())
        }
        None => (0, false, String::new()),
    };
    let poll = if show_gallery_on_main { None } else { list_topics(&state, Some("polls"), None, 0, 1).await?.into_iter().next() };
    let articles = if show_gallery_on_main { Vec::new() } else { list_topics(&state, Some("articles"), None, 0, 7).await? };
    let top_topics = all_topics.iter().take(10).cloned().collect();
    let mut gallery = Vec::new();
    if !show_gallery_on_main {
        for topic in list_topics(&state, Some("gallery"), None, 0, 12).await? {
            if let Some(image) = load_topic_images(&state, topic.id).await?.into_iter().next() {
                let srcset = image_srcset(&image);
                let padding_percent = 100.0 * f64::from(image.medium_height) / f64::from(image.medium_width);
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
    Ok(Html(MainPageTemplate {
        news,
        brief,
        add_url: add_reason.is_none().then(|| if show_gallery_on_main { "/add-section.jsp".to_string() } else { "/add-section.jsp?section=1".to_string() }),
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
    }.render()?))
}

pub async fn lenta(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some("forum"), None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    let navigation = build_topic_list_navigation(&state, "forum", None, &current_user).await?;
    Ok(Html(IndexTemplate { title: "Форум / лента".into(), topics, news, pager, main_page: false, tracker_layout: false, navigation: Some(navigation) }.render()?))
}

pub async fn section_topics(State(state): State<AppState>, uri: Uri, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    let navigation = build_topic_list_navigation(&state, section, None, &current_user).await?;
    Ok(Html(IndexTemplate { title: section_title(section).to_string(), topics, news, pager, main_page: false, tracker_layout: false, navigation: Some(navigation) }.render()?))
}

pub async fn section_group_topics(State(state): State<AppState>, uri: Uri, Path(group): Path<String>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), Some(&group), pager.offset, pager.limit).await?;
    let selected = crate::routes::groups::find_group(&state, &group).await?;
    if selected.section_prefix != section { return Err(AppError::NotFound); }
    let news = prepare_news_topics(&state, topics.clone(), false).await?;
    let navigation = build_topic_list_navigation(&state, section, Some(&selected), &current_user).await?;
    Ok(Html(IndexTemplate { title: format!("{} «{}»", section_title(section), selected.title), topics, news, pager, main_page: false, tracker_layout: false, navigation: Some(navigation) }.render()?))
}

pub async fn legacy_show_topics(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(_current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    let news = prepare_news_topics(&state, topics.clone(), true).await?;
    Ok(Html(IndexTemplate { title: "show-topics.jsp".into(), topics, news, pager, main_page: false, tracker_layout: false, navigation: None }.render()?))
}

const VIEW_ALL_SECTION_PREFIX_CASE: &str = "CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END";
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
        if self.id == GALLERY_SECTION_ID { "Неподтверждённые галереи".to_string() } else { format!("Неподтверждённые {}", self.name.to_lowercase()) }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct DeletedTopicRow {
    subj: String,
    nick: String,
    msgid: i32,
    reason: Option<String>,
    postdate: chrono::DateTime<chrono::Utc>,
    deldate: Option<chrono::NaiveDateTime>,
    bonus: Option<i32>,
}

impl DeletedTopicRow {
    fn reason_display(&self) -> &str {
        self.reason.as_deref().unwrap_or_default()
    }

    fn deldate_display(&self) -> String {
        self.deldate.map(|dt| dt.to_string()).unwrap_or_default()
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

/// Simplified AddTopicChecker.checkTopicPosting: only the postscore/karma
/// threshold and moderator-only/no-comments cases are modeled - frozen-user
/// and IP-block restrictions aren't checked here since this is just a UI
/// hint (the "Добавить" button), not an enforcement gate, and topic
/// creation itself doesn't enforce postscore restrictions yet either.
pub(crate) fn topic_posting_reason(restriction: i32, user: &Option<UserSummary>) -> Option<String> {
    let anonymous = user.is_none();
    let score = user.as_ref().and_then(|u| u.score).unwrap_or(0);
    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    match restriction {
        POSTSCORE_UNRESTRICTED => None,
        POSTSCORE_MODERATORS_ONLY => if is_moderator { None } else { Some("только для модераторов".to_string()) },
        POSTSCORE_REGISTERED_ONLY => if anonymous { Some("только для зарегистрированных".to_string()) } else { None },
        POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS => Some("постинг запрещен".to_string()),
        _ => if anonymous || score < restriction { Some(format!("только для зарегистрированных, score>={restriction}")) } else { None },
    }
}

/// The Java application can post as its dedicated anonymous user.  The Rust
/// port does not have that account/session path yet, so its navigation must
/// not advertise a link which inevitably ends in 403.
pub(crate) async fn posting_reason_for_port(state: &AppState, restriction: i32, user: &Option<UserSummary>) -> Result<Option<String>> {
    let Some(current) = user else {
        return Ok(Some("только для зарегистрированных".to_string()));
    };
    if current.blocked.unwrap_or(false) {
        return Ok(Some("аккаунт заблокирован".to_string()));
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
        .bind(current.id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();
    if frozen_until.is_some_and(|until| until > chrono::Utc::now()) {
        return Ok(Some("аккаунт заморожен".to_string()));
    }
    Ok(topic_posting_reason(restriction, user))
}

async fn build_topic_list_navigation(state: &AppState, section_prefix: &str, selected_group: Option<&Group>, user: &Option<UserSummary>) -> Result<TopicListNavigation> {
    let (section_id, section_restriction): (i32, i32) = sqlx::query_as(
        r#"SELECT id, restrict_score FROM sections WHERE CASE name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(name) END=$1"#,
    )
    .bind(section_prefix)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let groups = crate::routes::groups::list_groups_by_section(state, Some(section_prefix)).await?;
    let restriction = if let Some(group) = selected_group {
        let group_restriction: i32 = sqlx::query_scalar("SELECT COALESCE(restrict_topics, -9999) FROM groups WHERE id=$1")
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
    let quick_groups = groups.into_iter().map(|group| QuickGroupLink {
        title: group.title,
        url: format!("/{section_prefix}/{}", group.urlname),
        selected: selected_group.is_some_and(|selected| selected.id == group.id),
    }).collect();
    Ok(TopicListNavigation {
        section_url: Some(format!("/{section_prefix}/")),
        archive_url: (section_prefix != "forum").then(|| format!("/{section_prefix}/archive")),
        rss_url: Some(format!("/section-rss.jsp?section={section_id}")),
        add_url,
        add_reason: add_reason.unwrap_or_default(),
        moderator_group_id: user.as_ref().is_some_and(|u| u.canmod).then(|| selected_group.map(|g| g.id)).flatten(),
        quick_groups,
        all_groups_selected: selected_group.is_none(),
    })
}

/// UncommitedTopicsController/view-all.jsp: the premoderation queue -
/// public (no auth required, matching Java's `MaybeAuthorized`), lists
/// topics awaiting commit in premoderated sections plus recently deleted
/// ones, with an add-topic shortcut gated on posting permission.
pub async fn view_all(State(state): State<AppState>, Query(q): Query<ViewAllQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let section: Option<ViewAllSection> = if let Some(sid) = q.section.filter(|&id| id != 0) {
        let sql = format!("SELECT s.id, s.name, s.restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix FROM sections s WHERE s.id=$1");
        Some(sqlx::query_as::<_, ViewAllSection>(&sql).bind(sid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?)
    } else {
        None
    };

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE NOT t.deleted AND NOT t.draft AND t.moderate AND s.moderate
             AND t.postdate >= now() - interval '3 months'
             AND ($1::int IS NULL OR s.id=$1)
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC"#
    );
    let messages = sqlx::query_as::<_, TopicSummary>(&sql).bind(section.as_ref().map(|s| s.id)).fetch_all(&state.pool).await?;
    let uncommitted = messages.len() as i64;

    let bad_reason_filter = if is_moderator { "" } else { "AND di.reason != '' AND di.reason NOT IN ('Блокировка пользователя с удалением сообщений','4.6 Спам')" };
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
    let deleted_topics = sqlx::query_as::<_, DeletedTopicRow>(&sql).bind(section.as_ref().map(|s| s.id)).fetch_all(&state.pool).await?;

    let uncommitted_counts: Vec<(ViewAllSection, i64)> = if section.is_none() {
        let sql = format!(
            r#"SELECT s.id, s.name, s.restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix, count(t.*) AS cnt
               FROM sections s
               JOIN groups g ON g.section=s.id
               JOIN topics t ON t.groupid=g.id
               WHERE s.moderate AND NOT t.draft AND NOT t.deleted AND t.moderate
                 AND t.postdate >= now() - interval '3 months'
               GROUP BY s.id
               ORDER BY s.id"#
        );
        sqlx::query_as::<_, (i32, String, i32, String, i64)>(&sql)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(|(id, name, restrict_score, section_prefix, cnt)| (ViewAllSection { id, name, restrict_score, section_prefix }, cnt))
            .collect()
    } else {
        Vec::new()
    };

    let restriction = match &section {
        Some(s) => s.restrict_score,
        None => sqlx::query_scalar::<_, i32>("SELECT COALESCE(min(restrict_score), 0) FROM sections").fetch_one(&state.pool).await?,
    };
    let (add_link, add_link_reason) = match topic_posting_reason(restriction, &user) {
        None => (Some(match &section { Some(s) => format!("/add-section.jsp?section={}", s.id), None => "/add-section.jsp".to_string() }), None),
        Some(reason) => (None, Some(reason)),
    };

    let title = section.as_ref().map(|s| s.uncommited_name()).unwrap_or_else(|| "Просмотр неподтверждённых сообщений".to_string());
    let show_gallery_notice = section.as_ref().map(|s| s.id == GALLERY_SECTION_ID).unwrap_or(true);

    Ok(Html(ViewAllTemplate {
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
    }.render()?))
}

#[derive(Deserialize)]
pub struct ViewMessageQuery { msgid: i32 }

pub async fn legacy_view_message(Query(q): Query<ViewMessageQuery>) -> Redirect {
    Redirect::to(&format!("/jump-message.jsp?msgid={}", q.msgid))
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

pub async fn topic_page(State(state): State<AppState>, uri: Uri, Path((group, id)): Path<(String, i32)>, Query(q): Query<TopicViewQuery>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    render_topic_view(state, section, group, id, 0, None, q, current_user, csrf_token).await
}

pub async fn topic_page_with_page(State(state): State<AppState>, uri: Uri, Path((group, id, page_marker)): Path<(String, i32, String)>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let Some(page) = page_marker.strip_prefix("page") else { return Err(AppError::NotFound); };
    let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
    let section = section_from_uri(&uri).unwrap_or("forum");
    // Java's getMessagePage doesn't accept `cid`/`deleted`/`filter` at all -
    // only the base (page-less) route does.
    render_topic_view(state, section, group, id, page, None, TopicViewQuery::default(), current_user, csrf_token).await
}

pub async fn topic_thread(State(state): State<AppState>, uri: Uri, Path((group, id, thread_root)): Path<(String, i32, i32)>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    render_topic_view(state, section, group, id, 0, Some(thread_root), TopicViewQuery::default(), current_user, csrf_token).await
}

/// Called from legacy.rs's combined `/forum/{group}/{id_or_year}/{page_or_month}`
/// route once it's determined the third segment is `pageN`, not a year/month.
pub async fn render_topic_page(state: AppState, section: &'static str, group: String, id: i32, page: i64, current_user: Option<UserSummary>, csrf_token: String) -> Result<Response> {
    render_topic_view(state, section, group, id, page, None, TopicViewQuery::default(), current_user, csrf_token).await
}

async fn messages_per_page(state: &AppState, user: &Option<UserSummary>) -> i64 {
    match user {
        Some(u) => {
            let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
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
) -> Result<Response> {
    let topic = get_topic(&state, id).await?;
    let is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);

    // GroupPermissionService.checkView / drafts: a draft or not-yet-committed
    // premoderated topic is only visible to its author or a moderator. A
    // deleted topic is likewise author/moderator-only - the previous
    // implementation never checked `topic.deleted` at all here, so a
    // deleted topic stayed fully visible to everyone.
    if topic.draft || topic.moderate || topic.deleted {
        let allowed = current_user.as_ref().map(|u| u.canmod || u.id == topic.author_id).unwrap_or(false);
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
    let can_view_deleted_comments = allow_view_all_deleted_comments(&state, topic.id, &current_user).await?;
    if want_deleted && !can_view_deleted_comments {
        return Ok(Redirect::to(&topic.topic_url()).into_response());
    }

    // `?cid=` jumps straight to the comment (resolving its page), bypassing
    // the rest of rendering entirely - matches Java's inline jumpMessage
    // short-circuit in getMessageMain. Only the base (page-less, non-thread)
    // route wires this in.
    if let Some(cid) = query.cid {
        if thread_root.is_none() && page == 0 {
            return resolve_comment_jump(&state, &topic, cid, is_moderator, &current_user).await;
        }
    }

    let topic_html = markup::render_message_with_markup(&topic.message, Some(&topic.markup), topic.bbcode);

    let all_comments: Vec<CommentItem> = if want_deleted {
        topic_service(&state).vecListComments(id).await?
    } else {
        topic_service(&state).vecListComments(id).await?.into_iter().filter(|c| !c.deleted).collect()
    };

    // TopicController's hideSet: comments from ignored authors are dropped
    // from the rendered list (not just visually) unless `?filter=show`.
    let filter_show = query.filter.as_deref() == Some("show");
    let ignored_ids: Vec<i32> = match (&current_user, filter_show) {
        (Some(u), false) => sqlx::query_scalar("SELECT ignored FROM ignore_list WHERE userid=$1").bind(u.id).fetch_all(&state.pool).await.unwrap_or_default(),
        _ => vec![],
    };
    let unfiltered_count = all_comments.len();
    let visible_comments: Vec<CommentItem> = if ignored_ids.is_empty() {
        all_comments
    } else {
        all_comments.into_iter().filter(|c| !ignored_ids.contains(&c.author_id)).collect()
    };
    let filtered_count = visible_comments.len();

    let (page_comments, pages, thread_mode): (Vec<CommentItem>, Vec<CommentPageLink>, bool) = if let Some(root) = thread_root {
        let subtree = comment_subtree(&visible_comments, root);
        (subtree, vec![], true)
    } else if want_deleted {
        // Java's showDeleted path uses page=-1: render every comment on one
        // page, no pagination controls.
        (visible_comments, vec![], false)
    } else {
        let per_page = messages_per_page(&state, &current_user).await.max(1);
        let total_pages = (unfiltered_count as i64 + per_page - 1) / per_page.max(1);
        if page > 0 && page >= total_pages {
            let target_page = (total_pages - 1).max(0);
            let url = if target_page > 0 { format!("{}/page{target_page}", topic.topic_url()) } else { topic.topic_url() };
            return Ok(Redirect::to(&url).into_response());
        }
        let start = (page * per_page) as usize;
        let end = (start + per_page as usize).min(visible_comments.len());
        let slice = if start < visible_comments.len() { visible_comments[start..end].to_vec() } else { vec![] };
        let pages = if total_pages > 1 {
            (0..total_pages).map(|p| CommentPageLink { page: p, current: p == page }).collect()
        } else {
            vec![]
        };
        (slice, pages, false)
    };

    // ReactionService.allowInteract's expired/comments-hidden/frozen inputs,
    // fetched once up front so per-comment widgets don't each hit the DB.
    let (topic_expired, topic_postscore): (bool, i32) = sqlx::query_as(
        "SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, t.postscore FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1",
    )
    .bind(topic.id)
    .fetch_one(&state.pool)
    .await?;
    let comments_hidden = topic_postscore == POSTSCORE_HIDE_COMMENTS;
    let reactor_frozen = match &current_user {
        Some(u) => sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>("SELECT frozen_until FROM users WHERE id=$1")
            .bind(u.id).fetch_one(&state.pool).await?.map(|t| t > chrono::Utc::now()).unwrap_or(false),
        None => false,
    };
    let all_reactions = load_all_reactions(&state, topic.id).await?;
    let current_user_id = current_user.as_ref().map(|u| u.id);

    let comments: Vec<CommentView> = page_comments.into_iter().map(|item| {
        let html = markup::render_message_with_markup(&item.message, Some(&item.markup), item.bbcode);
        let rows: Vec<(String, i32, String, i32)> = all_reactions.iter()
            .filter(|(cid, ..)| *cid == Some(item.id))
            .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
            .collect();
        let allow_interact = reactions_allow_interact(&current_user, reactor_frozen, topic_expired, item.author_id, item.deleted, comments_hidden);
        let reactions = render_reactions_widget(topic.id, Some(item.id), &rows, current_user_id, allow_interact, &csrf_token);
        CommentView { item, html, reactions_html: reactions.html, show_reactions_link: reactions.show_menu_link }
    }).collect();

    let topic_reaction_rows: Vec<(String, i32, String, i32)> = all_reactions.iter()
        .filter(|(cid, ..)| cid.is_none())
        .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
        .collect();
    let topic_allow_interact = reactions_allow_interact(&current_user, reactor_frozen, topic_expired, topic.author_id, topic.deleted, false);
    let topic_reactions = render_reactions_widget(topic.id, None, &topic_reaction_rows, current_user_id, topic_allow_interact, &csrf_token);

    let poll = load_poll_view(&state, topic.id, topic.deleted, topic.moderate, topic_expired, query.results.unwrap_or(false), &current_user).await?;
    let images = load_topic_images(&state, topic.id).await?;
    let images_html = render_topic_images(&images, &topic.title, topic.section_prefix == "gallery", false);
    let (comment_format_mode, comment_format_title, _) = match &current_user {
        Some(user) => user_format_mode(&state, user.id).await?,
        None => (crate::profile::DEFAULT_FORMAT_MODE.into(), "Markdown".into(), "MARKDOWN".into()),
    };
    let can_comment = current_user.is_some() && !topic_expired && !topic.deleted && !comments_hidden;

    Ok(Html(TopicTemplate {
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
    }.render()?).into_response())
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
            if let Some(parent) = c.replyto {
                if ids.contains(&parent) {
                    ids.insert(c.id);
                }
            }
        }
        if ids.len() == before {
            break;
        }
    }
    let mut subtree: Vec<CommentItem> = comments.iter().filter(|c| ids.contains(&c.id)).cloned().collect();
    subtree.sort_by_key(|c| c.id);
    subtree
}

/// TopicController's inline `jumpMessage(msgid, cid, skipDeleted)`: resolves
/// which page a comment lives on (among non-deleted comments) and redirects
/// there with a `#comment-N` anchor; falls back to the deleted-comments view
/// for a moderator if the comment isn't found live.
async fn resolve_comment_jump(state: &AppState, topic: &TopicDetail, cid: i32, is_moderator: bool, current_user: &Option<UserSummary>) -> Result<Response> {
    let live_comments: Vec<CommentItem> = topic_service(state).vecListComments(topic.id).await?.into_iter().filter(|c| !c.deleted).collect();
    if let Some(pos) = live_comments.iter().position(|c| c.id == cid) {
        let per_page = messages_per_page(state, current_user).await.max(1);
        let page = pos as i64 / per_page;
        let url = if page > 0 { format!("{}/page{page}#comment-{cid}", topic.topic_url()) } else { format!("{}#comment-{cid}", topic.topic_url()) };
        return Ok(Redirect::to(&url).into_response());
    }
    if is_moderator {
        let exists_deleted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM comments WHERE id=$1 AND topic=$2 AND deleted)")
            .bind(cid)
            .bind(topic.id)
            .fetch_one(&state.pool)
            .await?;
        if exists_deleted {
            return Ok(Redirect::to(&format!("{}?deleted=true#comment-{cid}", topic.topic_url())).into_response());
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

pub async fn choose_topic_section(State(state): State<AppState>, Query(q): Query<NewTopicQuery>, CurrentUser(user): CurrentUser) -> Result<Response> {
    let tag = q.tags.or(q.tag).unwrap_or_default();
    if let Some(section_id) = q.section {
        let section_title: String = sqlx::query_scalar("SELECT name FROM sections WHERE id=$1")
            .bind(section_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
        let rows: Vec<(i32, String, String, Option<String>, i32, i32, String)> = sqlx::query_as(
            r#"SELECT g.id,g.title,g.urlname,g.info,COALESCE(g.restrict_topics,-9999),s.restrict_score,
                      CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END
               FROM groups g JOIN sections s ON s.id=g.section WHERE s.id=$1 ORDER BY g.title"#,
        ).bind(section_id).fetch_all(&state.pool).await?;
        let mut choices = Vec::with_capacity(rows.len());
        for (id, title, urlname, info, group_restriction, section_restriction, section_prefix) in rows {
            let reason = posting_reason_for_port(&state, group_restriction.max(section_restriction), &user).await?;
            let suffix = if tag.is_empty() { String::new() } else { format!("&tags={}", urlencoding::encode(&tag)) };
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
        return Ok(Html(AddSectionTemplate {
            title: format!("{section_title}: добавление"),
            heading: format!("Добавить в «{section_title}»"),
            choices,
            choosing_groups: true,
        }.render()?).into_response());
    }

    let rows: Vec<(i32, String, i32)> = sqlx::query_as("SELECT id,name,restrict_score FROM sections ORDER BY id").fetch_all(&state.pool).await?;
    let mut choices = Vec::with_capacity(rows.len());
    for (id, title, restriction) in rows {
        let reason = posting_reason_for_port(&state, restriction, &user).await?;
        let suffix = if tag.is_empty() { String::new() } else { format!("&tag={}", urlencoding::encode(&tag)) };
        choices.push(AddSectionChoice {
            title,
            url: format!("/add-section.jsp?section={id}{suffix}"),
            view_url: None,
            info: None,
            postable: reason.is_none(),
            reason: reason.unwrap_or_default(),
        });
    }
    Ok(Html(AddSectionTemplate {
        title: "Добавить топик".into(),
        heading: "Выберите раздел".into(),
        choices,
        choosing_groups: false,
    }.render()?).into_response())
}

struct TopicFormGroup {
    title: String,
    links_allowed: bool,
    poll_allowed: bool,
    image_required: bool,
    image_allowed_by_section: bool,
    section_prefix: String,
}

async fn load_topic_form_group(state: &AppState, group_id: i32) -> Result<TopicFormGroup> {
    let row: Option<(String, bool, bool, bool, bool, String)> = sqlx::query_as(
        r#"SELECT g.title, s.havelink, COALESCE(s.vote,false), s.imagepost,
                  (COALESCE(s.imageallowed,false) OR COALESCE(s.image_allowed,false)),
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    ).bind(group_id).fetch_optional(&state.pool).await?;
    let Some((title, links_allowed, poll_allowed, image_required, image_allowed_by_section, section_prefix)) = row else {
        return Err(AppError::NotFound);
    };
    Ok(TopicFormGroup { title, links_allowed, poll_allowed, image_required, image_allowed_by_section, section_prefix })
}

fn image_upload_allowed(group: &TopicFormGroup, user: &Option<UserSummary>) -> bool {
    group.image_required || (group.image_allowed_by_section && user.as_ref().is_some_and(|u| u.canmod || u.corrector || u.score.unwrap_or(0) >= 50))
}

pub async fn new_topic_form(State(state): State<AppState>, Query(q): Query<NewTopicQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let selected_group = match q.group {
        Some(id) => id,
        None => return Ok(Redirect::to("/add-section.jsp").into_response()),
    };
    let group = load_topic_form_group(&state, selected_group).await?;
    let (format_mode, format_mode_title, _) = match &user {
        Some(user) => user_format_mode(&state, user.id).await?,
        None => (crate::profile::DEFAULT_FORMAT_MODE.into(), "Markdown".into(), "MARKDOWN".into()),
    };
    let image_allowed = image_upload_allowed(&group, &user);
    let noinfo = q.noinfo.is_some();
    let initial_tags = q.tags.or(q.tag).unwrap_or_default();
    let add_info_html = if noinfo {
        None
    } else {
        let path = format!("{}/help/new-topic-{}.md", state.config.static_dir, group.section_prefix);
        tokio::fs::read_to_string(path).await.ok().map(|source| markup::render_markdown(&source))
    };
    Ok(Html(TopicFormTemplate {
        title: format!("Добавить в «{}»", group.title),
        action: "/add.jsp".into(),
        topic_id: None,
        csrf_token,
        poll_variants: Vec::new(),
        poll_new_rows: if group.poll_allowed { vec![String::new(); POLL_MAX_VARIANTS] } else { Vec::new() },
        poll_multiselect: false,
        selected_group,
        is_edit: false,
        links_allowed: group.links_allowed,
        poll_allowed: group.poll_allowed,
        image_allowed,
        image_required: group.image_required,
        additional_image_rows: if image_allowed && group.section_prefix != "forum" { vec![(); 3] } else { Vec::new() },
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
    }.render()?).into_response())
}

/// AddTopicController.MaxMessageLength (anonymous posting isn't supported by
/// Rust's session model, so only the registered-user limit applies).
const TOPIC_MAX_MESSAGE_LENGTH: usize = 65536;

struct TopicUpload {
    bytes: bytes::Bytes,
    original_name: Option<String>,
    primary: bool,
}

async fn parse_topic_request(state: &AppState, request: Request, expected_csrf: &str) -> Result<(Vec<(String, String)>, Vec<TopicUpload>)> {
    let multipart_request = request.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).is_some_and(|value| value.starts_with("multipart/form-data"));
    if !multipart_request {
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024).await.map_err(|error| AppError::BadRequest(format!("invalid body: {error}")))?;
        return Ok((crate::form::parse_pairs(&bytes)?, Vec::new()));
    }

    let mut multipart = Multipart::from_request(request, state).await.map_err(|error| AppError::BadRequest(format!("ошибка multipart: {error}")))?;
    let mut pairs = Vec::new();
    let mut uploads = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|error| AppError::BadRequest(format!("ошибка multipart: {error}")))? {
        let Some(name) = field.name().map(str::to_string) else { continue; };
        if name == "image" || name == "additionalImage" {
            let original_name = field.file_name().map(str::to_string);
            let bytes = field.bytes().await.map_err(|error| AppError::BadRequest(format!("ошибка чтения изображения: {error}")))?;
            if !bytes.is_empty() {
                uploads.push(TopicUpload { bytes, original_name, primary: name == "image" });
            }
        } else {
            let value = field.text().await.map_err(|error| AppError::BadRequest(format!("ошибка чтения поля {name}: {error}")))?;
            pairs.push((name, value));
        }
    }
    if crate::form::get(&pairs, "csrf") != Some(expected_csrf) {
        return Err(AppError::Forbidden);
    }
    Ok((pairs, uploads))
}

fn validate_topic_form(form: &TopicForm, links_allowed: bool) -> Result<()> {
    let title = form.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("заголовок сообщения не может быть пустым".into()));
    }
    if form.title.chars().count() > 140 {
        return Err(AppError::BadRequest("Слишком большой заголовок".into()));
    }
    if title.starts_with('[') {
        return Err(AppError::BadRequest("Не добавляйте теги в заголовки, используйте предназначенное для тегов поле ввода".into()));
    }
    if form.msg.chars().count() > TOPIC_MAX_MESSAGE_LENGTH {
        return Err(AppError::BadRequest("Слишком большое сообщение".into()));
    }
    if links_allowed {
        if let Some(url) = form.url.as_deref().filter(|value| !value.trim().is_empty()) {
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
    }
    Ok(())
}

fn validate_topic_image(data: &[u8]) -> Result<(image::DynamicImage, &'static str)> {
    use image::GenericImageView;
    const MAX_FILE_SIZE: usize = 8 * 1024 * 1024;
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком большой файл".into()));
    }
    let format = image::guess_format(data).map_err(|_| AppError::BadRequest("Некорректное изображение: неизвестный формат".into()))?;
    let extension = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::Gif => "gif",
        _ => return Err(AppError::BadRequest("Некорректное изображение: поддерживаются jpeg, gif и png".into())),
    };
    let image = image::load_from_memory_with_format(data, format).map_err(|error| AppError::BadRequest(format!("Некорректное изображение: {error}")))?;
    let (width, height) = image.dimensions();
    if !(400..=5120).contains(&width) || !(400..=5120).contains(&height) {
        return Err(AppError::BadRequest("Сбой загрузки изображения: недопустимые размеры изображения".into()));
    }
    if f64::from(height) / (f64::from(width) + 1.0) > 2.3 {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком узкое изображение".into()));
    }
    if f64::from(width) / (f64::from(height) + 1.0) > 5.0 {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком широкое изображение".into()));
    }
    Ok((image, extension))
}

async fn save_topic_upload(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, state: &AppState, topic_id: i32, user_id: i32, upload: &TopicUpload) -> Result<()> {
    use image::GenericImageView;
    let (image, extension) = validate_topic_image(&upload.bytes)?;
    let (width, height) = image.dimensions();
    let image_id: i32 = sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('images','id'))::int").fetch_one(&mut **tx).await?;
    let relative_dir = format!("images/{image_id}");
    let directory = format!("{}/{relative_dir}", state.config.upload_dir);
    tokio::fs::create_dir_all(&directory).await.map_err(|error| AppError::Anyhow(error.into()))?;
    tokio::fs::write(format!("{directory}/original.{extension}"), &upload.bytes).await.map_err(|error| AppError::Anyhow(error.into()))?;
    for size in [500u32, 1000, 1500, 2000] {
        let scaled = if width.max(height) <= size { image.clone() } else { image.resize(size, size, image::imageops::FilterType::Lanczos3) };
        let mut encoded = Vec::new();
        scaled.write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Jpeg).map_err(|error| AppError::Anyhow(error.into()))?;
        tokio::fs::write(format!("{directory}/{size}px.jpg"), encoded).await.map_err(|error| AppError::Anyhow(error.into()))?;
    }
    sqlx::query(
        "INSERT INTO images(id,userid,topic,original,medium,thumbnail,width,height,original_name,primary_image,extension,main) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$10)",
    ).bind(image_id).bind(user_id).bind(topic_id)
        .bind(format!("{relative_dir}/original.{extension}"))
        .bind(format!("{relative_dir}/1000px.jpg"))
        .bind(format!("{relative_dir}/500px.jpg"))
        .bind(width as i32).bind(height as i32).bind(&upload.original_name).bind(upload.primary).bind(extension)
        .execute(&mut **tx).await?;
    if upload.primary {
        sqlx::query("UPDATE topics SET image=$1 WHERE id=$2").bind(image_id).bind(topic_id).execute(&mut **tx).await?;
    }
    Ok(())
}

pub async fn create_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken, request: Request) -> Result<Response> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let (pairs, uploads) = parse_topic_request(&state, request, &csrf_token).await?;
    let form = parse_topic_form(&pairs)?;
    let (format_mode, format_mode_title, markup_id) = user_format_mode(&state, user.id).await?;
    let group = load_topic_form_group(&state, form.group).await?;
    validate_topic_form(&form, group.links_allowed)?;
    let is_draft = form.draft.is_some();
    let premoderated: bool = sqlx::query_scalar("SELECT s.moderate FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(form.group)
        .fetch_one(&state.pool).await?;
    let upload_allowed = image_upload_allowed(&group, &Some(user.clone()));
    if !uploads.is_empty() && !upload_allowed {
        return Err(AppError::Forbidden);
    }
    if group.image_required && !uploads.iter().any(|upload| upload.primary) {
        return Err(AppError::BadRequest("Изображение отсутствует".into()));
    }
    if uploads.iter().filter(|upload| upload.primary).count() > 1 || uploads.iter().filter(|upload| !upload.primary).count() > 3 {
        return Err(AppError::BadRequest("Слишком много изображений".into()));
    }

    if form.preview.is_some() {
        return Ok(Html(TopicFormTemplate {
            title: format!("Добавить в «{}»", group.title),
            action: "/add.jsp".into(),
            topic_id: None,
            csrf_token,
            poll_variants: Vec::new(),
            poll_new_rows: if group.poll_allowed { form.poll.clone() } else { Vec::new() },
            poll_multiselect: form.multiselect.is_some(),
            selected_group: form.group,
            is_edit: false,
            links_allowed: group.links_allowed,
            poll_allowed: group.poll_allowed,
            image_allowed: upload_allowed,
            image_required: group.image_required,
            additional_image_rows: if upload_allowed && group.section_prefix != "forum" { vec![(); 3] } else { Vec::new() },
            form_title: form.title.clone(),
            form_msg: form.msg.clone(),
            form_url: form.url.clone().unwrap_or_default(),
            form_linktext: form.linktext.clone().unwrap_or_default(),
            form_tags: form.tags.clone().unwrap_or_default(),
            preview_html: Some(markup::render_message_with_markup(&form.msg, Some(&markup_id), None)),
            noinfo: form.noinfo.as_deref().is_some_and(|value| matches!(value, "1" | "true" | "on")),
            add_info_html: None,
            format_mode: format_mode.clone(),
            format_mode_title: format_mode_title.clone(),
        }.render()?).into_response());
    }

    // AddTopicRequestValidator.validateTags/AddTopicController: every
    // topic needs 1-5 valid tags, and creating a brand-new tag (one that
    // doesn't already exist as a value or synonym) needs either a
    // premoderated section or score>=200 (GroupPermissionService.canCreateTag).
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(&state, &tags, &user, premoderated).await?;

    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    let id = service.iNextMessageId(&mut tx).await?;
    service.vInsertTopicMessage(&mut tx, id, &form.msg).await?;
    sqlx::query("UPDATE msgbase SET markup=$2, bbcode=$3 WHERE id=$1")
        .bind(id).bind(&markup_id).bind(markup_id != "MARKDOWN").execute(&mut *tx).await?;
    service.vInsertTopic(&mut tx, StNewTopic {
        iMsgId: id,
        iGroupId: form.group,
        iUserId: user.id,
        sTitle: form.title.trim(),
        optUrl: group.links_allowed.then_some(form.url.as_deref()).flatten().filter(|sValue| !sValue.trim().is_empty()),
        optLinkText: group.links_allowed.then_some(form.linktext.as_deref()).flatten().filter(|sValue| !sValue.trim().is_empty()),
        bDraft: is_draft,
        bPremoderated: premoderated,
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    if group.poll_allowed {
        // AddTopicController.preparePollPreview/TopicService.addMessage:
        // every submitted variant_id is 0 (new) on creation.
        let variant_ids = vec![0; form.poll.len()];
        save_poll(&mut tx, id, form.multiselect.is_some(), &variant_ids, &form.poll).await?;
    }
    for upload in &uploads {
        save_topic_upload(&mut tx, &state, id, user.id, upload).await?;
    }
    tx.commit().await?;
    notify_topic_created(&state, id, user.id, &form.msg).await?;
    crate::search_index::index_topic(&state, id, false).await;
    // Java shows a dedicated confirmation for protected sections because
    // the new topic is intentionally absent from the public section until
    // a moderator commits it.
    let topic = get_topic(&state, id).await?;
    if premoderated && !is_draft {
        return Ok(Html(ModeratedTopicTemplate { topic_url: topic.topic_url() }.render()?).into_response());
    }
    Ok(Redirect::to(&topic.topic_url()).into_response())
}

#[derive(Template)]
#[template(path = "topic_created_moderated.html")]
struct ModeratedTopicTemplate {
    topic_url: String,
}

pub async fn edit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let topic = get_topic(&state, q.msgid).await?;
    let selected_group = topic.group_id;
    let group = load_topic_form_group(&state, selected_group).await?;
    let (format_mode, format_mode_title) = markup_form_view(&topic.markup, topic.bbcode);
    let image_allowed = image_upload_allowed(&group, &user);
    let image_count: i64 = sqlx::query_scalar("SELECT count(*) FROM images WHERE topic=$1 AND NOT deleted AND NOT primary_image").bind(q.msgid).fetch_one(&state.pool).await?;
    // PollDao.getPollByTopicId/EditTopicController: pre-fill existing
    // variants (blank if the topic has no poll yet, e.g. a topic moved
    // into the Опросы section after creation) plus a handful of empty
    // slots for adding new ones.
    let poll_row: Option<(i32, bool)> = sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1").bind(q.msgid).fetch_optional(&state.pool).await?;
    let (poll_variants, poll_multiselect) = match poll_row {
        Some((poll_id, multiselect)) => {
            let variants = sqlx::query_as::<_, (i32, String)>("SELECT id, label FROM polls_variants WHERE vote=$1 ORDER BY id").bind(poll_id).fetch_all(&state.pool).await?;
            (variants, multiselect)
        }
        None => (Vec::new(), false),
    };
    Ok(Html(TopicFormTemplate {
        title: "Редактировать тему".into(),
        action: "/edit.jsp".into(),
        topic_id: Some(topic.id),
        csrf_token,
        poll_variants,
        poll_new_rows: if group.poll_allowed { vec![String::new(); POLL_NEW_VARIANT_SLOTS] } else { Vec::new() },
        poll_multiselect,
        selected_group,
        is_edit: true,
        links_allowed: group.links_allowed,
        poll_allowed: group.poll_allowed,
        image_allowed,
        image_required: false,
        additional_image_rows: if image_allowed && group.section_prefix != "forum" { vec![(); 3usize.saturating_sub(image_count as usize)] } else { Vec::new() },
        form_title: topic.title.clone(),
        form_msg: topic.message.clone(),
        form_url: topic.url.clone().unwrap_or_default(),
        form_linktext: topic.linktext.clone().unwrap_or_default(),
        form_tags: topic.tags_vec().join(", "),
        preview_html: None,
        noinfo: false,
        add_info_html: None,
        format_mode,
        format_mode_title,
    }.render()?))
}

/// Simplified from EditTopicChecker.checkContentEdit/checkEditByAuthor:
/// author (or moderator, unconditional bypass) may edit within a 14-day
/// window from posting, or at any time while still a draft. The corrector
/// role, premoderated-section/articles commitDate nuances, and the
/// postscore==NO_COMMENTS lock aren't modeled by Rust's session yet - this
/// intentionally errs toward Java's baseline author/moderator gate rather
/// than leaving the endpoint wide open.
const TOPIC_EDIT_WINDOW_DAYS: i64 = 14;

pub async fn edit_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken, request: Request) -> Result<Response> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let (pairs, uploads) = parse_topic_request(&state, request, &csrf_token).await?;
    let form = parse_topic_form(&pairs)?;
    let id = form.id.ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let meta = load_topic_delete_meta(&state, id).await?;
    let current_topic = get_topic(&state, id).await?;
    let group = load_topic_form_group(&state, current_topic.group_id).await?;
    validate_topic_form(&form, group.links_allowed)?;
    if meta.deleted {
        return Err(AppError::BadRequest("нельзя править удаленные топики".into()));
    }
    // EditTopicChecker.checkEditByAuthor: a draft is always editable by its
    // author; a committed, premoderated (non-Articles) topic is
    // *permanently* locked for the author, regardless of any deadline;
    // otherwise the 14-day window applies, measured from `commitDate` for
    // a committed Articles topic and from `postdate` everywhere else.
    let is_articles = current_topic.section_prefix == "articles";
    let permanently_locked = meta.commited && meta.premoderated && !is_articles;
    let deadline_base = if meta.commited && is_articles {
        meta.commitdate.map(|d| d.and_utc()).unwrap_or(meta.postdate)
    } else {
        meta.postdate
    };
    let editable_by_author = meta.author_id == user.id
        && (meta.draft || (!permanently_locked && chrono::Utc::now() <= deadline_base + chrono::Duration::days(TOPIC_EDIT_WINDOW_DAYS)));
    if !user.canmod && !editable_by_author {
        return Err(AppError::Forbidden);
    }
    let upload_allowed = image_upload_allowed(&group, &Some(user.clone()));
    if !uploads.is_empty() && !upload_allowed {
        return Err(AppError::Forbidden);
    }
    let additional_count: i64 = sqlx::query_scalar("SELECT count(*) FROM images WHERE topic=$1 AND NOT deleted AND NOT primary_image").bind(id).fetch_one(&state.pool).await?;
    if uploads.iter().filter(|upload| upload.primary).count() > 1 || additional_count + uploads.iter().filter(|upload| !upload.primary).count() as i64 > 3 {
        return Err(AppError::BadRequest("Слишком много изображений".into()));
    }

    // EditTopicRequestValidator.validateTags: same rule as topic creation.
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(&state, &tags, &user, meta.premoderated).await?;

    if form.preview.is_some() {
        let poll_variants = form.variant_id.iter().zip(form.poll.iter()).filter(|(id, _)| **id != 0).map(|(id, label)| (*id, label.clone())).collect();
        let poll_new_rows = form.variant_id.iter().zip(form.poll.iter()).filter(|(id, _)| **id == 0).map(|(_, label)| label.clone()).collect();
        return Ok(Html(TopicFormTemplate {
            title: "Редактирование".into(), action: "/edit.jsp".into(), topic_id: Some(id), csrf_token,
            poll_variants, poll_new_rows, poll_multiselect: form.multiselect.is_some(), selected_group: current_topic.group_id,
            is_edit: true, links_allowed: group.links_allowed, poll_allowed: group.poll_allowed,
            image_allowed: upload_allowed, image_required: false,
            additional_image_rows: if upload_allowed && group.section_prefix != "forum" { vec![(); 3usize.saturating_sub(additional_count as usize)] } else { Vec::new() },
            form_title: form.title.clone(), form_msg: form.msg.clone(), form_url: form.url.clone().unwrap_or_default(),
            form_linktext: form.linktext.clone().unwrap_or_default(), form_tags: form.tags.clone().unwrap_or_default(),
            preview_html: Some(markup::render_message_with_markup(&form.msg, Some(&current_topic.markup), current_topic.bbcode)),
            noinfo: false,
            add_info_html: None,
            format_mode: markup_form_view(&current_topic.markup, current_topic.bbcode).0,
            format_mode_title: markup_form_view(&current_topic.markup, current_topic.bbcode).1,
        }.render()?).into_response());
    }

    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    service.vUpdateTopicMessage(&mut tx, id, &form.msg).await?;
    service.vUpdateTopicHeader(&mut tx, StEditTopic {
        iMsgId: id,
        sTitle: form.title.trim(),
        optUrl: group.links_allowed.then_some(form.url).flatten(),
        optLinkText: group.links_allowed.then_some(form.linktext).flatten(),
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    if meta.poll_allowed && !form.variant_id.is_empty() {
        save_poll(&mut tx, id, form.multiselect.is_some(), &form.variant_id, &form.poll).await?;
    }
    if uploads.iter().any(|upload| upload.primary) {
        sqlx::query("UPDATE images SET deleted=true WHERE topic=$1 AND primary_image AND NOT deleted").bind(id).execute(&mut *tx).await?;
    }
    for upload in &uploads {
        save_topic_upload(&mut tx, &state, id, user.id, upload).await?;
    }
    tx.commit().await?;
    crate::search_index::index_topic(&state, id, false).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")).into_response())
}

#[derive(Deserialize)]
pub struct TopicActionForm { pub msgid: i32, pub resolve: Option<String>, pub reason: Option<String>, pub bonus: Option<i32> }

/// Matches GroupPermissionService.DeletePeriod: an author may delete their
/// own (non-draft, non-premoderated-and-committed) topic for 3 days after
/// posting, and only while it has no comments. Moderators bypass this.
const TOPIC_DELETE_WINDOW_HOURS: i64 = 72;

struct TopicDeleteMeta {
    author_id: i32,
    deleted: bool,
    postdate: chrono::DateTime<chrono::Utc>,
    commitdate: Option<chrono::NaiveDateTime>,
    draft: bool,
    premoderated: bool,
    commited: bool,
    comment_count: i64,
    poll_allowed: bool,
}

/// GroupPermissionService.canViewAllDeletedTopics: a listing-level "show me
/// deleted topics too" gate, distinct from (and much looser than) the
/// per-topic `ViewDeletedScore=200` in `check_topic_viewable` - any
/// authorized, non-frozen user with score>=50 qualifies, not just
/// moderators. No `SlowModeChecker` equivalent exists in this port, so
/// that extra restriction is not modeled.
pub(crate) async fn can_view_all_deleted_topics(state: &AppState, user: &Option<UserSummary>) -> Result<bool> {
    const CAN_VIEW_ALL_DELETED_SCORE: i32 = 50;
    let Some(user) = user else { return Ok(false); };
    // Java's canViewAllDeletedTopics has no isModerator special-case at
    // all - the score+frozen check applies uniformly, moderators included.
    if user.score.unwrap_or(0) < CAN_VIEW_ALL_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(user.id).fetch_optional(&state.pool).await?.flatten();
    Ok(!frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false))
}

/// TopicPermissionService.allowViewAllDeletedComments: the `?deleted=`
/// gate on a topic's own page - narrower than `can_view_all_deleted_topics`
/// (score>=200, not 50) but *does* bypass for moderators, unlike that one.
/// No `SlowModeChecker` equivalent exists in this port.
pub(crate) async fn allow_view_all_deleted_comments(state: &AppState, topic_id: i32, user: &Option<UserSummary>) -> Result<bool> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Ok(true);
    }
    const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
    const POSTSCORE_NO_COMMENTS: i32 = 10001;
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    let Some((expired, draft, postscore)): Option<(bool, bool, i32)> = sqlx::query_as(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, COALESCE(t.draft,false), t.postscore
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(false);
    };
    let topic_forbidden = expired || draft || matches!(postscore, POSTSCORE_MODERATORS_ONLY | POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS);
    if topic_forbidden {
        return Ok(false);
    }
    let Some(user) = user else { return Ok(false); };
    if user.score.unwrap_or(0) < VIEW_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(user.id).fetch_optional(&state.pool).await?.flatten();
    if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
        return Ok(false);
    }
    let score_loss: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE((SELECT sum(bonus) FROM del_info JOIN comments ON comments.id=del_info.msgid
             WHERE bonus IS NOT NULL AND bonus<>0 AND comments.userid<>2 AND comments.deleted AND topic=$1), 0)::int"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(score_loss < 150)
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
pub(crate) async fn check_topic_viewable(state: &AppState, topic_id: i32, user: &Option<UserSummary>) -> Result<()> {
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
            let deldate: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1").bind(topic_id).fetch_optional(&state.pool).await?.flatten();
            let delete_expired = deldate.map(|d| d.and_utc() < chrono::Utc::now() - chrono::Duration::days(VIEW_AFTER_DELETE_DAYS)).unwrap_or(true);
            if delete_expired {
                return Err(AppError::NotFound);
            }
            let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(current.id).fetch_optional(&state.pool).await?.flatten();
            if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
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
    let row: (i32, bool, chrono::DateTime<chrono::Utc>, Option<chrono::NaiveDateTime>, bool, bool, bool, i64, bool) = sqlx::query_as(
        r#"SELECT t.userid, t.deleted, t.postdate, t.commitdate, COALESCE(t.draft,false), s.moderate,
                  (t.commitdate IS NOT NULL), t.stat1::bigint, s.vote
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
        commitdate: row.3,
        draft: row.4,
        premoderated: row.5,
        commited: row.6,
        comment_count: row.7,
        poll_allowed: row.8,
    })
}

pub async fn delete_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if meta.deleted {
        return Err(AppError::BadRequest("сообщение уже удалено".into()));
    }

    let deletable_by_author = meta.author_id == user.id && (
        meta.draft || (
            !(meta.premoderated && meta.commited)
                && meta.comment_count == 0
                && chrono::Utc::now() <= meta.postdate + chrono::Duration::hours(TOPIC_DELETE_WINDOW_HOURS)
        )
    );
    // GroupPermissionService.isDeletable: an administrator always passes;
    // otherwise try the author path first, and only fall back to
    // isDeletableByModerator (which itself refuses a committed
    // premoderated topic more than a month old, admin-only past that
    // point) when the author path fails and the actor is a moderator.
    let deletable = if user.candel {
        true
    } else if deletable_by_author {
        true
    } else if user.canmod {
        !meta.premoderated
            || !meta.commited
            || chrono::Utc::now() <= meta.postdate + chrono::Duration::days(30)
    } else {
        false
    };
    if !deletable {
        return Err(AppError::Forbidden);
    }

    let bonus = if user.canmod && user.id != meta.author_id && !meta.draft {
        form.bonus.unwrap_or(0).clamp(0, 20)
    } else {
        0
    };
    let reason = form.reason.clone().unwrap_or_default();

    topic_service(&state).vSetDeleted(form.msgid, true).await?;
    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&state.pool).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1").bind(meta.author_id).bind(bonus).execute(&state.pool).await?;
    }
    crate::routes::comments::notify_deleted(&state, meta.author_id, user.id, Some(form.msgid), None, &reason).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to("/"))
}

pub async fn undelete_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod {
        return Err(AppError::Forbidden);
    }
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if !meta.deleted {
        return Err(AppError::BadRequest("сообщение не удалено".into()));
    }
    // GroupPermissionService.isUndeletable: an administrator can always
    // undelete; a plain moderator only while the topic isn't expired, or -
    // if it is - within 14 days of the deletion itself.
    if !user.candel {
        let expired = crate::routes::comments::is_topic_expired(&state, form.msgid).await?;
        if expired {
            let deldate: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1").bind(form.msgid).fetch_optional(&state.pool).await?.flatten();
            let recently_deleted = deldate.map(|d| d.and_utc() > chrono::Utc::now() - chrono::Duration::days(14)).unwrap_or(false);
            if !recently_deleted {
                return Err(AppError::Forbidden);
            }
        }
    }
    topic_service(&state).vSetDeleted(form.msgid, false).await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1").bind(form.msgid).execute(&state.pool).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    // Java: `new ModelAndView(new RedirectView(topic.getLink))` - a topic
    // (not a comment), so no ?cid= here.
    let topic = get_topic(&state, form.msgid).await?;
    Ok(Redirect::to(&topic.topic_url()))
}

pub async fn resolve_topic_get(State(state): State<AppState>, Query(form): Query<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

pub async fn resolve_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

async fn do_resolve_topic(state: &AppState, user: Option<crate::models::UserSummary>, form: TopicActionForm) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let Some((author_id, group_resolvable)) = topic_service(state).optResolveMeta(form.msgid).await? else {
        return Err(AppError::NotFound);
    };
    if !group_resolvable {
        return Err(AppError::Forbidden);
    }
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }
    let resolved = form.resolve.as_deref().map(|value| value == "yes");
    topic_service(state).vSetResolved(form.msgid, resolved).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn list_topics(state: &AppState, section: Option<&str>, group: Option<&str>, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    topic_service(state).vecListTopics(section, group, offset, limit).await
}

pub async fn get_topic(state: &AppState, id: i32) -> Result<TopicDetail> {
    topic_service(state).stGetTopic(id).await
}


fn topic_service(state: &AppState) -> CTopicService<CTopicPgRepository> {
    CTopicService::new(CTopicPgRepository::new(state.pool.clone()))
}

/// PollDao.createPoll/updatePoll unified into one helper: creates the
/// topic's poll row on first call, then on every call reconciles
/// `polls_variants` against the submitted (variant_id, label) pairs -
/// `variant_id==0` inserts a new variant, an existing id with an empty
/// label deletes it, an existing id with a non-empty label updates it.
/// `variant_id` is scoped to `vote=voteid` in every UPDATE/DELETE so a
/// forged id from another poll can't be touched.
async fn save_poll(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, topic_id: i32, multiselect: bool, variant_ids: &[i32], labels: &[String]) -> Result<()> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM polls WHERE topic=$1").bind(topic_id).fetch_optional(&mut **tx).await?;
    let voteid = match existing {
        Some(id) => {
            sqlx::query("UPDATE polls SET multiselect=$1 WHERE id=$2").bind(multiselect).bind(id).execute(&mut **tx).await?;
            id
        }
        None => {
            let id: i32 = sqlx::query_scalar("SELECT nextval('vote_id')::int").fetch_one(&mut **tx).await?;
            sqlx::query("INSERT INTO polls(id, multiselect, topic) VALUES($1,$2,$3)").bind(id).bind(multiselect).bind(topic_id).execute(&mut **tx).await?;
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
            sqlx::query("DELETE FROM polls_variants WHERE id=$1 AND vote=$2").bind(variant_id).bind(voteid).execute(&mut **tx).await?;
        } else {
            sqlx::query("UPDATE polls_variants SET label=$1 WHERE id=$2 AND vote=$3").bind(label).bind(variant_id).bind(voteid).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

/// TopicService.sendEvents: on topic creation, notify (a) users mentioned
/// via @nick in the body (REF), and (b) users who favorited one of the
/// topic's tags (TAG) - excluding anyone already notified via (a), matching
/// Java's `tagUsers.filterNot(userRefIds.contains)`. Java also records
/// `topic_users_notified` to suppress duplicate notifications across a
/// later edit; this port doesn't re-run sendEvents on edit at all, so that
/// bookkeeping table is skipped as unnecessary for now.
async fn notify_topic_created(state: &AppState, topic_id: i32, author_id: i32, message: &str) -> Result<()> {
    let mentioned_nicks = markup::extract_mentions(message);
    let mut notified: Vec<i32> = if mentioned_nicks.is_empty() {
        vec![]
    } else {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick) = ANY($1) AND u.id <> $2
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(mentioned_nicks.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>())
        .bind(author_id)
        .fetch_all(&state.pool)
        .await?
    };
    for &mentioned_id in &notified {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'REF',false,$2)")
            .bind(mentioned_id)
            .bind(topic_id)
            .execute(&state.pool)
            .await?;
    }

    let tag_favoriters: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT ut.userid FROM user_tags ut
           JOIN tags tg ON tg.tagid=ut.tag_id
           WHERE tg.msgid=$1 AND ut.is_favorite AND ut.userid<>$2 AND NOT ut.userid=ANY($3)"#,
    )
    .bind(topic_id)
    .bind(author_id)
    .bind(&notified)
    .fetch_all(&state.pool)
    .await?;
    for &tag_userid in &tag_favoriters {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'TAG',false,$2)")
            .bind(tag_userid)
            .bind(topic_id)
            .execute(&state.pool)
            .await?;
    }
    notified.extend(tag_favoriters);

    if !notified.is_empty() {
        notified.sort_unstable();
        notified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&notified)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
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

pub async fn delete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить тему #{}</h1>
<form method="post" action="/delete.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Удалить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn undelete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Восстановить тему #{}</h1>
<form method="post" action="/undelete">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#, q.msgid, q.msgid)))
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

pub async fn commit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1").bind(q.msgid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
    Ok(Html(format!(r#"
<h1>Подтвердить тему #{}</h1>
<form method="post" action="/commit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Подтвердить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1").bind(form.msgid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
    topic_service(&state).vCommitTopic(form.msgid, user.id).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn uncommit_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Отменить подтверждение темы #{}</h1>
<form method="post" action="/uncommit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Отменить подтверждение</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn uncommit(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vUncommitTopic(form.msgid).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct MoveTopicForm { pub msgid: i32, pub moveto: i32 }

pub async fn move_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    let mut options = String::new();
    for g in groups {
        let selected = if g.id == topic.group_id { " selected" } else { "" };
        options.push_str(&format!("<option value=\"{}\"{}>{} / {}</option>", g.id, selected, html_escape::encode_text(&g.section_name), html_escape::encode_text(&g.title)));
    }
    Ok(Html(format!(r#"
<h1>Переместить тему #{}</h1>
<form method="post" action="/mt.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <select name="moveto">{}</select>
  <button type="submit">Переместить</button>
</form>
"#, q.msgid, q.msgid, options)))
}

pub async fn move_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<MoveTopicForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vMoveTopic(form.msgid, form.moveto).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn premoderated_move_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, user: CurrentUser, csrf: crate::csrf::CsrfToken) -> Result<Html<String>> {
    move_topic_form(State(state), Query(q), user, csrf).await
}
