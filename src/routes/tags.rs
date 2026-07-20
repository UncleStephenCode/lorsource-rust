use crate::{auth::CurrentUser, error::{AppError, Result}, models::{TagItem, TopicSummary}, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::{Html, IntoResponse, Redirect}, Form, Json};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "tags.html")]
struct TagsTemplate {
    title: String,
    tags: Vec<TagItem>,
}

#[derive(Debug, Clone)]
struct TagSectionGroup {
    section_prefix: String,
    section_name: String,
    topics: Vec<TopicSummary>,
    add_url: Option<String>,
    add_reason: String,
    add_label: String,
}

#[derive(Template)]
#[template(path = "tag_page.html")]
struct TagPageTemplate {
    tag: String,
    title: String,
    counter: i64,
    sections: Vec<TagSectionGroup>,
    synonyms: Vec<String>,
    show_favorite_button: bool,
    show_unfavorite_button: bool,
    show_ignore_button: bool,
    show_unignore_button: bool,
    show_delete: bool,
    current_user: Option<crate::models::UserSummary>,
    csrf_token: String,
    favorites_count: i64,
    ignored_count: i64,
}

#[derive(Deserialize)]
pub struct AllTagsQuery { pub term: Option<String> }

/// TagController's `/tags` path is shared with `showTagListHandlerJSON`,
/// disambiguated in Java by `params = Array("term")`; axum has no
/// path+query-based dispatch, so branch on `term`'s presence here instead.
pub async fn all_tags(State(state): State<AppState>, Query(q): Query<AllTagsQuery>) -> Result<axum::response::Response> {
    if let Some(term) = q.term.filter(|t| !t.is_empty()) {
        return Ok(Json(tag_autocomplete(&state, &term).await?).into_response());
    }
    let tags = sqlx::query_as::<_, TagItem>("SELECT value,counter FROM tags_values ORDER BY lower(value) LIMIT 1000")
        .fetch_all(&state.pool).await?;
    Ok(Html(TagsTemplate { title: "Метки".into(), tags }.render()?).into_response())
}

/// TagService.suggestTagsByPrefix/TagDao.getTopTagsByPrefix: union of real
/// tag values and synonyms matching `prefix%` with counter>=2, top 10 by
/// counter, alphabetically sorted, then filtered by `isGoodTag` in the
/// controller.
async fn tag_autocomplete(state: &AppState, term: &str) -> Result<Vec<String>> {
    let escaped = term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
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

pub async fn tags_by_letter(State(state): State<AppState>, Path(first_letter): Path<String>) -> Result<Html<String>> {
    let prefix = format!("{}%", first_letter);
    let tags = sqlx::query_as::<_, TagItem>("SELECT value,counter FROM tags_values WHERE lower(value) LIKE lower($1) ORDER BY lower(value) LIMIT 1000")
        .bind(prefix).fetch_all(&state.pool).await?;
    Ok(Html(TagsTemplate { title: format!("Метки: {first_letter}"), tags }.render()?))
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
    ("gallery", "Галерея"),
    ("forum", "Форум"),
    ("polls", "Опросы"),
    ("articles", "Статьи"),
];

/// TagPageController.tagPage: aggregates the tag's topics across all 5
/// sections (news/gallery/forum/polls/articles) on one page instead of a
/// flat single-section list, resolves a synonym redirect if the tag itself
/// has no direct topics, lists sibling synonyms, and surfaces
/// favorite/ignore-tag button state - none of which the previous flat
/// listing did.
pub async fn tag_page(State(state): State<AppState>, Path(tag): Path<String>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;

    if !is_good_tag(&tag) {
        return Err(AppError::NotFound);
    }

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    let tag_row: Option<(i32, i64)> = sqlx::query_as(
        "SELECT id, counter::bigint FROM tags_values WHERE lower(value)=lower($1)",
    )
    .bind(&tag)
    .fetch_optional(&state.pool)
    .await?;

    let Some((tag_id, counter)) = tag_row.filter(|(_, counter)| is_moderator || *counter > 0) else {
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
            Some(main_tag) => Ok(Redirect::to(&format!("/tag/{}", urlencoding::encode(&main_tag))).into_response()),
            None => Err(AppError::NotFound),
        };
    };

    let mut sections = Vec::new();
    for (prefix, name) in TAG_SECTION_ORDER {
        let limit = section_topic_limit(prefix);
        let (section_id, restriction): (i32, i32) = sqlx::query_as(
            r#"SELECT id,restrict_score FROM sections WHERE CASE name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(name) END=$1"#,
        ).bind(prefix).fetch_one(&state.pool).await?;
        let topics = sqlx::query_as::<_, TopicSummary>(
            r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                      g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                      s.id AS section_id, s.name AS section_name,
                      $1::text AS section_prefix,
                      t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                      string_agg(tv2.value, ',' ORDER BY tv2.value) AS tags
               FROM topics t
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               JOIN tags tg ON tg.msgid=t.id AND tg.tagid=$2
               LEFT JOIN tags tg2 ON tg2.msgid=t.id
               LEFT JOIN tags_values tv2 ON tv2.id=tg2.tagid
               WHERE (CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END) = $1
                 AND NOT t.deleted AND NOT COALESCE(t.draft,false) AND ($3 OR NOT t.moderate)
               GROUP BY t.id,u.id,g.id,s.id
               ORDER BY t.postdate DESC LIMIT $4"#,
        )
        .bind(prefix)
        .bind(tag_id)
        .bind(is_moderator)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?;
        if !topics.is_empty() {
            let add_reason = crate::routes::topics::posting_reason_for_port(&state, restriction, &user).await?;
            let add_url = add_reason.is_none().then(|| format!("/add-section.jsp?section={section_id}&tag={}", urlencoding::encode(&tag)));
            let add_label = match *prefix {
                "news" => "Добавить новость",
                "gallery" => "Добавить изображение",
                "polls" => "Добавить опрос",
                _ => "Добавить топик",
            }.to_string();
            sections.push(TagSectionGroup { section_prefix: prefix.to_string(), section_name: name.to_string(), topics, add_url, add_reason: add_reason.unwrap_or_default(), add_label });
        }
    }

    let synonyms: Vec<String> = sqlx::query_scalar("SELECT value FROM tags_synonyms WHERE tagid=$1 ORDER BY value")
        .bind(tag_id)
        .fetch_all(&state.pool)
        .await?;

    let (show_favorite_button, show_unfavorite_button, show_ignore_button, show_unignore_button) = match &user {
        Some(u) => {
            let is_fav: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_tags WHERE userid=$1 AND tag_id=$2 AND is_favorite)")
                .bind(u.id).bind(tag_id).fetch_one(&state.pool).await?;
            let is_ignored: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_tags WHERE userid=$1 AND tag_id=$2 AND NOT is_favorite)")
                .bind(u.id).bind(tag_id).fetch_one(&state.pool).await?;
            (!is_fav, is_fav, !is_moderator && !is_ignored, !is_moderator && is_ignored)
        }
        None => (false, false, false, false),
    };
    let favorites_count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite").bind(tag_id).fetch_one(&state.pool).await?;
    let ignored_count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND NOT is_favorite").bind(tag_id).fetch_one(&state.pool).await?;

    Ok(Html(TagPageTemplate {
        tag: tag.clone(),
        title: format!("Метка: {}", capitalize_first(&tag)),
        counter,
        sections,
        synonyms,
        show_favorite_button,
        show_unfavorite_button,
        show_ignore_button,
        show_unignore_button,
        show_delete: is_moderator,
        current_user: user,
        csrf_token,
        favorites_count,
        ignored_count,
    }.render()?).into_response())
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
static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^[\p{L}\d-](?:[.\p{L}\d \+-]*[\p{L}\d\+-])?$").expect("tag regex"));

fn is_good_tag(tag: &str) -> bool {
    let len = tag.chars().count();
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
        return Err(format!("Слишком много тегов (максимум {MAX_TAGS_PER_TOPIC})"));
    }
    if good.is_empty() {
        return Err("Установите теги".to_string());
    }
    Ok(good)
}

/// GroupPermissionService.canCreateTag: outside a premoderated section, a
/// user needs score>=200 to mint a brand-new tag; inside one, any
/// authenticated (non-anonymous) user may. Checked only against tags that
/// don't already exist (TagService.getNewTags) - applying an existing tag
/// never requires this.
pub(crate) async fn check_can_create_new_tags(state: &AppState, tags: &[String], user: &crate::models::UserSummary, section_premoderated: bool) -> Result<()> {
    if section_premoderated {
        return Ok(());
    }
    if user.score.unwrap_or(0) >= CREATE_TAG_SCORE {
        return Ok(());
    }
    let mut new_tags = Vec::new();
    for tag in tags {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tags_values WHERE lower(value)=lower($1)) OR EXISTS(SELECT 1 FROM tags_synonyms WHERE lower(value)=lower($1))",
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
        Err(AppError::BadRequest(format!("Вы не можете создавать новые теги ({})", new_tags.join(", "))))
    }
}

fn first_letter_of(tag: &str) -> String {
    tag.chars().next().map(|c| c.to_lowercase().to_string()).unwrap_or_default()
}

async fn get_tag_id(pool: &sqlx::PgPool, name: &str) -> Result<Option<i32>> {
    Ok(sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)").bind(name).fetch_optional(pool).await?)
}

#[derive(Deserialize)]
pub struct TagChangeQuery {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: String,
}

pub async fn change_form(CurrentUser(user): CurrentUser, Query(q): Query<TagChangeQuery>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Переименовать метку</h1>
<form method="post" action="/tags/change" class="form">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="firstLetter" value="{fl}">
<label>Старая <input name="oldTagName" value="{old}" required readonly></label>
<label>Новая <input name="tagName" value="{old}" required></label>
<button type="submit">Переименовать</button>
</form>
"#,
        fl = html_escape::encode_double_quoted_attribute(q.first_letter.as_deref().unwrap_or("")),
        old = html_escape::encode_double_quoted_attribute(&q.tag_name),
    )))
}

#[derive(Deserialize)]
pub struct TagChangeForm {
    #[serde(rename = "oldTagName")]
    pub old_tag_name: String,
    #[serde(rename = "tagName")]
    pub tag_name: String,
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
}

pub async fn change_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TagChangeForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let old_tag_name = form.old_tag_name.trim();
    let tag_name = form.tag_name.trim();

    let Some(old_tag_id) = get_tag_id(&state.pool, old_tag_name).await? else {
        return Err(AppError::BadRequest("Тега с таким именем не существует!".into()));
    };
    if !is_good_tag(tag_name) {
        return Err(AppError::BadRequest(format!("Некорректный тег: '{tag_name}'")));
    }
    if get_tag_id(&state.pool, tag_name).await?.is_some() {
        return Err(AppError::BadRequest("Тег с таким именем уже существует!".into()));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM tags_synonyms WHERE value=$1").bind(tag_name).execute(&mut *tx).await?;
    sqlx::query("UPDATE tags_values SET value=$2 WHERE id=$1").bind(old_tag_id).bind(tag_name).execute(&mut *tx).await?;
    tx.commit().await?;

    // TagModificationService.change calls searchQueueSender.updateMessage
    // for every topic carrying this tag - the indexed tag value would
    // otherwise stay stale under the old name forever.
    reindex_topics_with_tag(&state, old_tag_id).await;

    Ok(Redirect::to(&format!("/tags/{}", urlencoding::encode(&first_letter_of(tag_name)))))
}

async fn reindex_topics_with_tag(state: &AppState, tag_id: i32) {
    let topic_ids: Vec<i32> = sqlx::query_scalar("SELECT msgid FROM tags WHERE tagid=$1")
        .bind(tag_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
    for id in topic_ids {
        crate::search_index::index_topic(state, id, true).await;
    }
}

#[derive(Deserialize)]
pub struct TagDeleteQuery {
    #[serde(rename = "firstLetter")]
    pub first_letter: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: String,
}

pub async fn delete_form(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<TagDeleteQuery>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let is_synonym: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags_synonyms WHERE value=$1)")
        .bind(q.tag_name.trim())
        .fetch_one(&state.pool)
        .await?;
    let synonym_note = if is_synonym {
        "<p class=\"muted\">Это синоним - будет удалена только сама ссылка-синоним.</p>"
    } else {
        ""
    };
    Ok(Html(format!(r#"
<h1>Удалить метку</h1>
{synonym_note}
<form method="post" action="/tags/delete" class="form">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="firstLetter" value="{fl}">
<input type="hidden" name="oldTagName" value="{old}">
<p>Удаляемая метка: <b>{old}</b></p>
<label>Заменить на (оставьте пустым, чтобы просто удалить) <input name="tagName"></label>
<label><input type="checkbox" name="createSynonym" value="true"> Оставить синоним на новую метку</label>
<button type="submit">Удалить</button>
</form>
"#,
        fl = html_escape::encode_double_quoted_attribute(q.first_letter.as_deref().unwrap_or("")),
        old = html_escape::encode_text(&q.tag_name),
        synonym_note = synonym_note,
    )))
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

pub async fn delete_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TagDeleteForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let old_tag_name = form.old_tag_name.trim();

    // A synonym entry isn't a real tag - deleting it just drops the redirect.
    let synonym_target: Option<i32> = sqlx::query_scalar("SELECT tagid FROM tags_synonyms WHERE value=$1").bind(old_tag_name).fetch_optional(&state.pool).await?;
    if synonym_target.is_some() {
        sqlx::query("DELETE FROM tags_synonyms WHERE value=$1").bind(old_tag_name).execute(&state.pool).await?;
        return Ok(Redirect::to(&format!("/tags/{}", urlencoding::encode(&first_letter_of(old_tag_name)))));
    }

    let Some(old_tag_id) = get_tag_id(&state.pool, old_tag_name).await? else {
        return Err(AppError::BadRequest("Тега с таким именем не существует!".into()));
    };
    let tag_name = form.tag_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let create_synonym = form.create_synonym.is_some();

    let Some(tag_name) = tag_name else {
        if create_synonym {
            return Err(AppError::BadRequest("Не указан тег для создания синонима!".into()));
        }
        let affected_topics: Vec<i32> = sqlx::query_scalar("SELECT msgid FROM tags WHERE tagid=$1").bind(old_tag_id).fetch_all(&state.pool).await?;
        let mut tx = state.pool.begin().await?;
        sqlx::query("DELETE FROM user_tags WHERE tag_id=$1").bind(old_tag_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM tags WHERE tagid=$1").bind(old_tag_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM tags_synonyms WHERE tagid=$1").bind(old_tag_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM tags_values WHERE id=$1").bind(old_tag_id).execute(&mut *tx).await?;
        tx.commit().await?;
        // TagModificationService.delete: reindex every topic that lost the tag.
        for id in affected_topics {
            crate::search_index::index_topic(&state, id, true).await;
        }
        return Ok(Redirect::to(&format!("/tags/{}", urlencoding::encode(&first_letter_of(old_tag_name)))));
    };

    if !is_good_tag(tag_name) {
        return Err(AppError::BadRequest(format!("Некорректный тег: '{tag_name}'")));
    }
    if old_tag_name.eq_ignore_ascii_case(tag_name) {
        return Err(AppError::BadRequest("Заменяемый тег не должен быть равен удаляемому!".into()));
    }

    let mut tx = state.pool.begin().await?;
    let new_tag_id: i32 = sqlx::query_scalar(
        "INSERT INTO tags_values(value,counter) VALUES($1,0) ON CONFLICT(value) DO UPDATE SET value=EXCLUDED.value RETURNING id",
    )
    .bind(tag_name)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO tags(msgid,tagid) SELECT msgid,$2 FROM tags WHERE tagid=$1 ON CONFLICT DO NOTHING")
        .bind(old_tag_id).bind(new_tag_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM tags WHERE tagid=$1").bind(old_tag_id).execute(&mut *tx).await?;

    sqlx::query("INSERT INTO user_tags(userid,tag_id,is_favorite) SELECT userid,$2,is_favorite FROM user_tags WHERE tag_id=$1 ON CONFLICT DO NOTHING")
        .bind(old_tag_id).bind(new_tag_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM user_tags WHERE tag_id=$1").bind(old_tag_id).execute(&mut *tx).await?;

    sqlx::query("UPDATE tags_values SET counter=(SELECT count(*) FROM tags WHERE tagid=$1) WHERE id=$1").bind(new_tag_id).execute(&mut *tx).await?;

    // Any synonym that pointed at the tag being removed now follows the merge target.
    sqlx::query("UPDATE tags_synonyms SET tagid=$2 WHERE tagid=$1").bind(old_tag_id).bind(new_tag_id).execute(&mut *tx).await?;
    if create_synonym {
        sqlx::query("INSERT INTO tags_synonyms(value,tagid) VALUES($1,$2) ON CONFLICT(value) DO UPDATE SET tagid=EXCLUDED.tagid")
            .bind(old_tag_name).bind(new_tag_id).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM tags_values WHERE id=$1").bind(old_tag_id).execute(&mut *tx).await?;
    tx.commit().await?;

    // TagModificationService.merge: reindex every topic now carrying the merge target's tag.
    reindex_topics_with_tag(&state, new_tag_id).await;

    Ok(Redirect::to(&format!("/tags/{}", urlencoding::encode(&first_letter_of(tag_name)))))
}
