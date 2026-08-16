use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    domain::user::statistics::{
        StUserStatisticsLocalData, StUserStatisticsSection, TrUserStatisticsLocalRepository,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct CUserStatisticsPgRepository {
    oPool: PgPool,
}

impl CUserStatisticsPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

fn sSectionUrlName(iSectionId: i32) -> Result<&'static str> {
    match iSectionId {
        1 => Ok("news"),
        2 => Ok("forum"),
        3 => Ok("gallery"),
        5 => Ok("polls"),
        6 => Ok("articles"),
        _ => Err(AppError::Anyhow(anyhow::anyhow!(
            "unknown section id {iSectionId}"
        ))),
    }
}

#[async_trait]
impl TrUserStatisticsLocalRepository for CUserStatisticsPgRepository {
    async fn stLocalData(&self, iUserId: i32) -> Result<StUserStatisticsLocalData> {
        // These are the only synchronous/database values used by Java's
        // UserStatisticsService.  In particular, do not count comments or
        // topics here: those values intentionally come from OpenSearch.
        let fCommentDates = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
            "SELECT min(postdate), max(postdate) FROM comments WHERE userid=$1",
        )
        .bind(iUserId)
        .fetch_one(&self.oPool);
        let fIgnoreCount = sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*)::bigint
                 FROM ignore_list il
                 JOIN users u ON u.id=il.userid
                WHERE il.ignored=$1 AND NOT u.blocked"#,
        )
        .bind(iUserId)
        .fetch_one(&self.oPool);
        let fSections =
            sqlx::query_as::<_, (i32, String)>("SELECT id,name FROM sections ORDER BY id")
                .fetch_all(&self.oPool);

        let ((optFirstComment, optLastComment), iIgnoreCount, vecSectionRows) =
            tokio::try_join!(fCommentDates, fIgnoreCount, fSections)?;
        let vecSections = vecSectionRows
            .into_iter()
            .map(|(iId, sName)| {
                Ok(StUserStatisticsSection {
                    iId,
                    sName,
                    // Section.getUrlName is a fixed ID mapping in Java and
                    // throws for unknown IDs.  Preserve that strictness.
                    sUrlName: sSectionUrlName(iId)?.to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(StUserStatisticsLocalData {
            iIgnoreCount,
            optFirstComment,
            optLastComment,
            vecSections,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_section_ids_match_java_url_names() {
        assert_eq!(sSectionUrlName(1).unwrap(), "news");
        assert_eq!(sSectionUrlName(2).unwrap(), "forum");
        assert_eq!(sSectionUrlName(3).unwrap(), "gallery");
        assert_eq!(sSectionUrlName(5).unwrap(), "polls");
        assert_eq!(sSectionUrlName(6).unwrap(), "articles");
        assert!(matches!(sSectionUrlName(4), Err(AppError::Anyhow(_))));
    }

    #[test]
    fn source_contract_keeps_counts_in_opensearch() {
        let sSource = include_str!("user_statistics_repository.rs");
        assert!(sSource.contains("SELECT min(postdate), max(postdate) FROM comments"));
        assert!(sSource.contains("il.ignored=$1 AND NOT u.blocked"));
        let sDeletedFilter = ["NOT COALESCE(", "deleted"].concat();
        assert!(!sSource.contains(&sDeletedFilter));
        let sTopicTable = ["FROM ", "topics"].concat();
        assert!(!sSource.contains(&sTopicTable));
    }
}
