use async_trait::async_trait;
use sqlx::{FromRow, PgPool};

use crate::{
    domain::topic::options::{
        POSTSCORE_UNRESTRICTED, StSetTopicOptions, StTopicOptions, TrTopicOptionsRepository,
    },
    error::Result,
};

const S_TOPIC_OPTIONS_SQL: &str = r#"
SELECT t.id AS i_topic_id,
       COALESCE(t.postscore,-9999) AS i_post_score,
       t.sticky AS b_sticky,
       t.notop AS b_no_top,
       s.moderate AS b_premoderated,
       CASE s.id
         WHEN 1 THEN 'news'
         WHEN 2 THEN 'forum'
         WHEN 3 THEN 'gallery'
         WHEN 5 THEN 'polls'
         WHEN 6 THEN 'articles'
         ELSE lower(s.name)
       END AS s_section_prefix,
       g.urlname AS s_group_url_name
  FROM topics t
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
 WHERE t.id=$1
"#;

const S_SET_TOPIC_OPTIONS_SQL: &str = r#"
UPDATE topics
   SET postscore=$2,
       sticky=$3,
       notop=$4,
       lastmod=CURRENT_TIMESTAMP
 WHERE id=$1
"#;

#[derive(Debug, FromRow)]
struct StTopicOptionsRow {
    i_topic_id: i32,
    i_post_score: Option<i32>,
    b_sticky: bool,
    b_no_top: bool,
    b_premoderated: bool,
    s_section_prefix: String,
    s_group_url_name: String,
}

impl From<StTopicOptionsRow> for StTopicOptions {
    fn from(stRow: StTopicOptionsRow) -> Self {
        Self {
            iTopicId: stRow.i_topic_id,
            iPostScore: stRow.i_post_score.unwrap_or(POSTSCORE_UNRESTRICTED),
            bSticky: stRow.b_sticky,
            bNoTop: stRow.b_no_top,
            bPremoderated: stRow.b_premoderated,
            sCanonicalUrl: format!(
                "/{}/{}/{}",
                stRow.s_section_prefix, stRow.s_group_url_name, stRow.i_topic_id
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CTopicOptionsPgRepository {
    oPool: PgPool,
}

impl CTopicOptionsPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrTopicOptionsRepository for CTopicOptionsPgRepository {
    async fn optFind(&self, iTopicId: i32) -> Result<Option<StTopicOptions>> {
        Ok(sqlx::query_as::<_, StTopicOptionsRow>(S_TOPIC_OPTIONS_SQL)
            .bind(iTopicId)
            .fetch_optional(&self.oPool)
            .await?
            .map(Into::into))
    }

    async fn vSet(&self, stOptions: StSetTopicOptions) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        sqlx::query(S_SET_TOPIC_OPTIONS_SQL)
            .bind(stOptions.iTopicId)
            .bind(stOptions.iPostScore)
            .bind(stOptions.bSticky)
            .bind(stOptions.bNoTop)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_deleted_draft_and_expired_topics_without_extra_state_guards() {
        assert!(!S_TOPIC_OPTIONS_SQL.contains("t.deleted"));
        assert!(!S_TOPIC_OPTIONS_SQL.contains("t.draft"));
        assert!(!S_TOPIC_OPTIONS_SQL.contains("expired"));
        assert!(S_TOPIC_OPTIONS_SQL.contains("s.moderate AS b_premoderated"));
    }

    #[test]
    fn mutation_is_the_single_unconditional_java_update_of_all_options_and_lastmod() {
        for sFragment in [
            "postscore=$2",
            "sticky=$3",
            "notop=$4",
            "lastmod=CURRENT_TIMESTAMP",
        ] {
            assert!(S_SET_TOPIC_OPTIONS_SQL.contains(sFragment), "{sFragment}");
        }
        assert!(!S_SET_TOPIC_OPTIONS_SQL.contains("IS DISTINCT FROM"));
        assert!(!S_SET_TOPIC_OPTIONS_SQL.contains("FOR UPDATE"));
    }

    #[test]
    fn canonical_link_uses_the_constrained_group_url_name_as_a_path_segment() {
        let stOptions: StTopicOptions = StTopicOptionsRow {
            i_topic_id: 42,
            i_post_score: Some(-9999),
            b_sticky: false,
            b_no_top: false,
            b_premoderated: false,
            s_section_prefix: "forum".into(),
            s_group_url_name: "linux-org-ru".into(),
        }
        .into();
        assert_eq!(stOptions.sCanonicalUrl, "/forum/linux-org-ru/42");
    }
}
