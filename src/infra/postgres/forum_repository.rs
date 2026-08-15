use async_trait::async_trait;
use sqlx::PgPool;

use crate::domain::forum::{model::StGroup, repository::TrForumRepository};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct CForumPgRepository {
    oPool: PgPool,
}

impl CForumPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrForumRepository for CForumPgRepository {
    async fn vecListGroupsBySection(&self, optSectionPrefix: Option<&str>) -> Result<Vec<StGroup>> {
        Ok(sqlx::query_as::<_, StGroup>(
            sqlx::AssertSqlSafe(
                S_GROUP_SELECT_SQL.to_string()
                    + " WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END=$1) GROUP BY g.id,s.id ORDER BY s.id,g.id",
            ),
        )
        .bind(optSectionPrefix)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn stFindGroupBySectionAndUrlName(
        &self,
        sSectionPrefix: &str,
        sUrlName: &str,
    ) -> Result<StGroup> {
        Ok(sqlx::query_as::<_, StGroup>(sqlx::AssertSqlSafe(
            S_GROUP_SELECT_SQL.to_string()
                + " WHERE CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END=$1 AND g.urlname=$2 GROUP BY g.id,s.id",
        ))
        .bind(sSectionPrefix)
        .bind(sUrlName)
        .fetch_one(&self.oPool)
        .await?)
    }
}

const S_GROUP_SELECT_SQL: &str = r#"
SELECT g.id, g.title, g.urlname, g.section, s.name AS section_name,
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
       g.info, g.longinfo, count(t.id) AS topics, g.stat3 AS topics_per_day
FROM groups g
JOIN sections s ON s.id=g.section
LEFT JOIN topics t ON t.groupid=g.id AND NOT t.deleted
"#;
