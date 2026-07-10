use crate::{auth::CurrentUser, application::topic::CTopicService, domain::topic::repository::{StEditTopic, StNewTopic}, error::{AppError, Result}, infra::postgres::topic_repository::CTopicPgRepository, markup, models::{PagerQuery, TopicDetail, TopicSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, http::Uri, response::{Html, Redirect}, Form};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Template)]
#[template(path = "topic.html")]
struct TopicTemplate {
    topic: TopicDetail,
    topic_html: String,
    comments: Vec<CommentView>,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Debug, Clone)]
struct CommentView {
    item: crate::models::CommentItem,
    html: String,
}

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
    action: String,
    topic: Option<TopicDetail>,
    groups: Vec<crate::models::Group>,
}

#[derive(Deserialize)]
pub struct TopicForm {
    pub id: Option<i32>,
    pub group: i32,
    pub title: String,
    pub msg: String,
    pub url: Option<String>,
    pub linktext: Option<String>,
    pub tags: Option<String>,
}

pub async fn index(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "Последние темы".into(), topics, pager, current_user }.render()?))
}

pub async fn lenta(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some("forum"), None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "Форум / лента".into(), topics, pager, current_user }.render()?))
}

pub async fn section_topics(State(state): State<AppState>, uri: Uri, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: section_title(section).to_string(), topics, pager, current_user }.render()?))
}

pub async fn section_group_topics(State(state): State<AppState>, uri: Uri, Path(group): Path<String>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), Some(&group), pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: format!("{} / {}", section_title(section), group), topics, pager, current_user }.render()?))
}

pub async fn legacy_show_topics(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "show-topics.jsp".into(), topics, pager, current_user }.render()?))
}

#[derive(Deserialize)]
pub struct ViewMessageQuery { msgid: i32 }

pub async fn legacy_view_message(Query(q): Query<ViewMessageQuery>) -> Redirect {
    Redirect::to(&format!("/jump-message.jsp?msgid={}", q.msgid))
}

pub async fn topic_page(State(state): State<AppState>, Path((_group, id)): Path<(String, i32)>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    render_topic(state, id, current_user).await
}

pub async fn topic_page_with_page(State(state): State<AppState>, Path((_group, id, page_marker)): Path<(String, i32, String)>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let Some(page) = page_marker.strip_prefix("page") else { return Err(AppError::NotFound); };
    let _page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
    render_topic(state, id, current_user).await
}

pub async fn render_topic(state: AppState, id: i32, current_user: Option<crate::models::UserSummary>) -> Result<Html<String>> {
    let topic = get_topic(&state, id).await?;
    let topic_html = markup::render_message(&topic.message, topic.bbcode);
    let items = topic_service(&state).vecListComments(id).await?;
    let comments = items.into_iter().map(|item| CommentView { html: markup::render_message(&item.message, Some(true)), item }).collect();
    Ok(Html(TopicTemplate { topic, topic_html, comments, current_user }.render()?))
}

pub async fn new_topic_form(State(state): State<AppState>) -> Result<Html<String>> {
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Новая тема".into(), action: "/add.jsp".into(), topic: None, groups }.render()?))
}

pub async fn create_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    let id = service.iNextMessageId(&mut tx).await?;
    service.vInsertTopicMessage(&mut tx, id, &form.msg).await?;
    service.vInsertTopic(&mut tx, StNewTopic {
        iMsgId: id,
        iGroupId: form.group,
        iUserId: user.id,
        sTitle: form.title.trim(),
        optUrl: form.url.as_deref().filter(|sValue| !sValue.trim().is_empty()),
        optLinkText: form.linktext.as_deref().filter(|sValue| !sValue.trim().is_empty()),
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

pub async fn edit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>) -> Result<Html<String>> {
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Редактировать тему".into(), action: "/edit.jsp".into(), topic: Some(topic), groups }.render()?))
}

pub async fn edit_topic(State(state): State<AppState>, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let id = form.id.ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    service.vUpdateTopicMessage(&mut tx, id, &form.msg).await?;
    service.vUpdateTopicHeader(&mut tx, StEditTopic {
        iMsgId: id,
        sTitle: form.title.trim(),
        optUrl: form.url,
        optLinkText: form.linktext,
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

#[derive(Deserialize)]
pub struct TopicActionForm { pub msgid: i32, pub resolve: Option<String> }

pub async fn delete_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    topic_service(&state).vSetDeleted(form.msgid, true).await?;
    Ok(Redirect::to("/"))
}

pub async fn undelete_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    topic_service(&state).vSetDeleted(form.msgid, false).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn resolve_topic_get(State(state): State<AppState>, Query(form): Query<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

pub async fn resolve_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
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

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    uri.path().trim_start_matches('/').split('/').next().and_then(|s| match s {
        "forum" | "news" | "articles" | "gallery" | "polls" => Some(s),
        _ => None,
    })
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

pub async fn delete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить тему #{}</h1>
<form method="post" action="/delete.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Удалить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn undelete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Восстановить тему #{}</h1>
<form method="post" action="/undelete">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Подтвердить тему #{}</h1>
<form method="post" action="/commit.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Подтвердить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod { return Err(AppError::Forbidden); }
    topic_service(&state).vCommitTopic(form.msgid, user.id).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn uncommit_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Отменить подтверждение темы #{}</h1>
<form method="post" action="/uncommit.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Отменить подтверждение</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn uncommit(State(state): State<AppState>, Form(form): Form<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vUncommitTopic(form.msgid).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct MoveTopicForm { pub msgid: i32, pub moveto: i32 }

pub async fn move_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
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
  <input type="hidden" name="msgid" value="{}">
  <select name="moveto">{}</select>
  <button type="submit">Переместить</button>
</form>
"#, q.msgid, q.msgid, options)))
}

pub async fn move_topic(State(state): State<AppState>, Form(form): Form<MoveTopicForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vMoveTopic(form.msgid, form.moveto).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn premoderated_move_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, user: CurrentUser) -> Result<Html<String>> {
    move_topic_form(State(state), Query(q), user).await
}
