use crate::{error::Result, models::{SearchQuery, TopicSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Query, State}, response::Html};

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    q: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
}

pub async fn search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Result<Html<String>> {
    let query = q.q.unwrap_or_default();
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = if query.trim().is_empty() {
        vec![]
    } else {
        sqlx::query_as::<_, TopicSummary>(
            r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                      g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                      s.id AS section_id, s.name AS section_name,
                      CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                      t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                      string_agg(tv.value, ',' ORDER BY tv.value) AS tags
               FROM topics t
               JOIN msgbase m ON m.id=t.id
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               LEFT JOIN tags tg ON tg.msgid=t.id
               LEFT JOIN tags_values tv ON tv.id=tg.tagid
               WHERE NOT t.deleted AND (to_tsvector('russian', t.title || ' ' || m.message) @@ plainto_tsquery('russian', $1) OR t.title ILIKE '%' || $1 || '%')
               GROUP BY t.id,u.id,g.id,s.id
               ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
        ).bind(&query).bind(pager.offset).bind(pager.limit).fetch_all(&state.pool).await?
    };
    Ok(Html(SearchTemplate { q: query, topics, pager }.render()?))
}
