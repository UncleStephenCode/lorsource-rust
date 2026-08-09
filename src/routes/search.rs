use crate::{
    error::{AppError, Result},
    request_timezone::stRequestTimezone,
    search_index::{
        self, FacetItem, MAX_OFFSET, SEARCH_ROWS, SearchInterval, SearchItem, SearchParams,
        SearchRange, SearchSort, SearchTag,
    },
    state::AppState,
};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    q: String,
    user: String,
    usertopic: bool,
    dt: i64,
    selected_date: String,
    section: String,
    sort: String,
    interval: String,
    range: String,
    error: Option<String>,
    items: Vec<SearchItem>,
    total: i64,
    shown_count: i64,
    took_ms: i64,
    section_facet: Vec<FacetItem>,
    group_facet: Vec<FacetItem>,
    found_tags: Vec<SearchTag>,
    prev_link: Option<String>,
    next_link: Option<String>,
    searched: bool,
    debug: bool,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub section: Option<String>,
    pub group: Option<String>,
    pub user: Option<String>,
    pub usertopic: Option<String>,
    pub sort: Option<String>,
    pub interval: Option<String>,
    pub range: Option<String>,
    pub offset: Option<String>,
    pub dt: Option<String>,
    pub debug: Option<String>,
}

fn parse_sort(s: Option<&str>) -> std::result::Result<SearchSort, String> {
    match s.unwrap_or("RELEVANCE") {
        "RELEVANCE" | "1" | "" => Ok(SearchSort::Relevance),
        "DATE" | "2" => Ok(SearchSort::Date),
        "DATE_OLD_TO_NEW" => Ok(SearchSort::DateOldToNew),
        value => Err(format!("Неверное значение sort: {value}")),
    }
}

fn sort_id(s: SearchSort) -> &'static str {
    match s {
        SearchSort::Relevance => "RELEVANCE",
        SearchSort::Date => "DATE",
        SearchSort::DateOldToNew => "DATE_OLD_TO_NEW",
    }
}

fn parse_interval(s: Option<&str>) -> std::result::Result<SearchInterval, String> {
    match s.unwrap_or("ALL").to_uppercase().as_str() {
        "MONTH" => Ok(SearchInterval::Month),
        "THREE_MONTH" => Ok(SearchInterval::ThreeMonth),
        "YEAR" => Ok(SearchInterval::Year),
        "THREE_YEAR" => Ok(SearchInterval::ThreeYear),
        "ALL" | "" => Ok(SearchInterval::All),
        value => Err(format!("Неверное значение interval: {value}")),
    }
}

fn interval_id(i: SearchInterval) -> &'static str {
    match i {
        SearchInterval::Month => "MONTH",
        SearchInterval::ThreeMonth => "THREE_MONTH",
        SearchInterval::Year => "YEAR",
        SearchInterval::ThreeYear => "THREE_YEAR",
        SearchInterval::All => "ALL",
    }
}

fn parse_range(s: Option<&str>) -> std::result::Result<SearchRange, String> {
    match s.unwrap_or("ALL").to_uppercase().as_str() {
        "TOPICS" => Ok(SearchRange::Topics),
        "COMMENTS" => Ok(SearchRange::Comments),
        "ALL" | "" => Ok(SearchRange::All),
        value => Err(format!("Неверное значение range: {value}")),
    }
}

fn range_id(r: SearchRange) -> &'static str {
    match r {
        SearchRange::All => "ALL",
        SearchRange::Topics => "TOPICS",
        SearchRange::Comments => "COMMENTS",
    }
}

fn optSelectedDayBounds(iDt: i64, stTimezone: chrono_tz::Tz) -> Option<(i64, i64, String)> {
    if iDt <= 0 {
        return None;
    }
    let dtSelected = Utc.timestamp_millis_opt(iDt).single()?;
    let stDate = dtSelected.with_timezone(&stTimezone).date_naive();
    let dtStart = stTimezone
        .from_local_datetime(&stDate.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    let dtEnd = stTimezone
        .from_local_datetime(&stDate.succ_opt()?.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    Some((
        dtStart.timestamp_millis(),
        dtEnd.timestamp_millis(),
        stDate.format("%d.%m.%Y").to_string(),
    ))
}

async fn sanitize_section_group(
    stState: &AppState,
    sSection: String,
    sGroup: String,
) -> Result<(String, String)> {
    if sSection.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let optSection: Option<(i32, String)> = sqlx::query_as(
        r#"SELECT s.id,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                    WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                    WHEN 6 THEN 'articles' ELSE lower(s.name) END
             FROM sections s
            WHERE CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                    WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                    WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1
               OR s.id::text = $1"#,
    )
    .bind(&sSection)
    .fetch_optional(&stState.pool)
    .await?;
    let Some((iSectionId, sCanonicalSection)) = optSection else {
        return Ok((String::new(), String::new()));
    };
    if sGroup.is_empty() {
        return Ok((sCanonicalSection, String::new()));
    }
    let optCanonicalGroup: Option<String> = if sGroup.chars().all(|c| c.is_ascii_digit()) {
        sqlx::query_scalar("SELECT urlname FROM groups WHERE section=$1 AND id::text=$2")
            .bind(iSectionId)
            .bind(&sGroup)
            .fetch_optional(&stState.pool)
            .await?
    } else if sGroup
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        sqlx::query_scalar("SELECT urlname FROM groups WHERE section=$1 AND urlname=$2")
            .bind(iSectionId)
            .bind(&sGroup)
            .fetch_optional(&stState.pool)
            .await?
    } else {
        None
    };
    Ok((sCanonicalSection, optCanonicalGroup.unwrap_or_default()))
}

fn sQueryLink(
    sQuery: &str,
    enRange: SearchRange,
    enInterval: SearchInterval,
    optUser: Option<&str>,
    bUserTopic: bool,
    enSort: SearchSort,
    sSection: &str,
    sGroup: &str,
    iOffset: i64,
) -> String {
    let mut vecParts = Vec::new();
    if !sQuery.is_empty() {
        vecParts.push(format!("q={}", urlencoding::encode(sQuery)));
        vecParts.push(format!("oldQ={}", urlencoding::encode(sQuery)));
    }
    if enRange != SearchRange::All {
        vecParts.push(format!("range={}", range_id(enRange)));
    }
    if enInterval != SearchInterval::All {
        vecParts.push(format!("interval={}", interval_id(enInterval)));
    }
    if let Some(sUser) = optUser {
        vecParts.push(format!("user={}", urlencoding::encode(sUser)));
    }
    if bUserTopic {
        vecParts.push("usertopic=true".to_owned());
    }
    if enSort != SearchSort::Relevance {
        vecParts.push(format!("sort={}", sort_id(enSort)));
    }
    if !sSection.is_empty() {
        vecParts.push(format!("section={}", urlencoding::encode(sSection)));
    }
    // SearchServiceRequest.getQuery includes group whenever it is non-null;
    // sanitized requests always contain a (possibly empty) group string.
    vecParts.push(format!("group={}", urlencoding::encode(sGroup)));
    if iOffset != 0 {
        vecParts.push(format!("offset={iOffset}"));
    }
    format!("/search.jsp?{}", vecParts.join("&"))
}

/// SearchController.search / SearchService.performSearch, including the
/// original binding, sanitization, date and pagination contracts.
pub async fn search(
    State(stState): State<AppState>,
    stJar: CookieJar,
    Query(stQuery): Query<SearchQuery>,
) -> Result<Html<String>> {
    let sQueryText = stQuery.q.unwrap_or_default();
    let sRequestedUser = stQuery.user.unwrap_or_default();
    let bUserTopic = stQuery.usertopic.as_deref() == Some("true");
    let stTimezone = stRequestTimezone(&stJar);
    let mut optError = None;

    let enSort = parse_sort(stQuery.sort.as_deref()).unwrap_or_else(|sError| {
        optError = Some(sError);
        SearchSort::Relevance
    });
    let enInterval = parse_interval(stQuery.interval.as_deref()).unwrap_or_else(|sError| {
        optError.get_or_insert(sError);
        SearchInterval::All
    });
    let enRange = parse_range(stQuery.range.as_deref()).unwrap_or_else(|sError| {
        optError.get_or_insert(sError);
        SearchRange::All
    });
    let iOffset = stQuery
        .offset
        .as_deref()
        .unwrap_or("0")
        .parse::<i64>()
        .unwrap_or_else(|_| {
            optError.get_or_insert_with(|| "Неверное значение offset".to_owned());
            0
        });
    let iDt = stQuery
        .dt
        .as_deref()
        .unwrap_or("0")
        .parse::<i64>()
        .unwrap_or_else(|_| {
            optError.get_or_insert_with(|| "Неверное значение dt".to_owned());
            0
        });
    let optDay = optSelectedDayBounds(iDt, stTimezone);
    if iDt > 0 && optDay.is_none() {
        optError.get_or_insert_with(|| "Неверное значение dt".to_owned());
    }

    let optCanonicalUser: Option<String> = if sRequestedUser.is_empty() {
        None
    } else {
        let optNick: Option<String> =
            sqlx::query_scalar("SELECT nick FROM users WHERE lower(nick)=lower($1)")
                .bind(&sRequestedUser)
                .fetch_optional(&stState.pool)
                .await?;
        if optNick.is_none() {
            optError
                .get_or_insert_with(|| format!("Пользователь \"{sRequestedUser}\" не существует"));
        }
        optNick
    };

    let bInitial = sQueryText.is_empty() && optCanonicalUser.is_none() && iDt <= 0;
    let bSearched = !bInitial && optError.is_none();
    let sRequestedSection = stQuery.section.unwrap_or_default();
    let sRequestedGroup = stQuery.group.unwrap_or_default();
    let (mut sSection, sGroup) = if bSearched {
        sanitize_section_group(&stState, sRequestedSection, sRequestedGroup).await?
    } else {
        // SearchController calls sanitizeQuery only inside its successful
        // non-initial binding branch. Preserve rejected/initial form values.
        (sRequestedSection, sRequestedGroup)
    };

    let mut vecItems = Vec::new();
    let mut iTotal = 0;
    let mut iTookMs = 0;
    let mut vecSectionFacet = Vec::new();
    let mut vecGroupFacet = Vec::new();
    let mut vecFoundTags = Vec::new();
    let mut optPrevLink = None;
    let mut optNextLink = None;

    if bSearched {
        let stParams = SearchParams {
            q: sQueryText.clone(),
            section: Some(sSection.clone()).filter(|sValue| !sValue.is_empty()),
            group: Some(sGroup.clone()).filter(|sValue| !sValue.is_empty()),
            user: optCanonicalUser.clone(),
            usertopic: bUserTopic,
            sort: enSort,
            interval: enInterval,
            range: enRange,
            offset: iOffset,
            selected_day_ms: optDay.as_ref().map(|(iStart, iEnd, _)| (*iStart, *iEnd)),
            timezone: stTimezone,
        };
        let stResult = search_index::search(&stState, &stParams)
            .await
            .map_err(|sError| AppError::Anyhow(anyhow::anyhow!(sError)))?;
        sSection = stResult.effective_section;
        vecItems = stResult.items;
        iTotal = stResult.total;
        iTookMs = stResult.took_ms;
        vecSectionFacet = stResult.section_facet;
        vecGroupFacet = stResult.group_facet;
        vecFoundTags = stResult.found_tags;

        let iNextOffset = iOffset + SEARCH_ROWS;
        if iNextOffset < MAX_OFFSET && iTotal > iNextOffset {
            optNextLink = Some(sQueryLink(
                &sQueryText,
                enRange,
                enInterval,
                optCanonicalUser.as_deref(),
                bUserTopic,
                enSort,
                &sSection,
                &sGroup,
                iNextOffset,
            ));
        }
        if iOffset - SEARCH_ROWS >= 0 {
            optPrevLink = Some(sQueryLink(
                &sQueryText,
                enRange,
                enInterval,
                optCanonicalUser.as_deref(),
                bUserTopic,
                enSort,
                &sSection,
                &sGroup,
                iOffset - SEARCH_ROWS,
            ));
        }
    }

    Ok(Html(
        SearchTemplate {
            q: sQueryText,
            user: optCanonicalUser.unwrap_or(sRequestedUser),
            usertopic: bUserTopic,
            dt: iDt,
            selected_date: optDay.map(|(_, _, sDate)| sDate).unwrap_or_default(),
            section: sSection,
            sort: sort_id(enSort).to_owned(),
            interval: interval_id(enInterval).to_owned(),
            range: range_id(enRange).to_owned(),
            error: optError,
            shown_count: vecItems.len() as i64,
            items: vecItems,
            total: iTotal,
            took_ms: iTookMs,
            section_facet: vecSectionFacet,
            group_facet: vecGroupFacet,
            found_tags: vecFoundTags,
            prev_link: optPrevLink,
            next_link: optNextLink,
            searched: bSearched,
            debug: stQuery.debug.is_some(),
        }
        .render()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_numeric_sort_ids_remain_compatible() {
        assert_eq!(parse_sort(Some("1")).unwrap(), SearchSort::Relevance);
        assert_eq!(parse_sort(Some("2")).unwrap(), SearchSort::Date);
        assert!(parse_sort(Some("3")).is_err());
    }

    #[test]
    fn selected_day_uses_request_timezone_and_dst_length() {
        let stTimezone: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let iNoon = Utc
            .with_ymd_and_hms(2025, 3, 30, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        let (iStart, iEnd, sDate) = optSelectedDayBounds(iNoon, stTimezone).unwrap();
        assert_eq!(sDate, "30.03.2025");
        assert_eq!(iEnd - iStart, 23 * 60 * 60 * 1000);
    }

    #[test]
    fn pagination_matches_java_and_intentionally_omits_selected_date() {
        let sLink = sQueryLink(
            "rust search",
            SearchRange::Topics,
            SearchInterval::All,
            Some("tester"),
            true,
            SearchSort::Date,
            "forum",
            "linux-org-ru",
            25,
        );
        assert!(sLink.contains("q=rust%20search&oldQ=rust%20search"));
        assert!(sLink.contains("group=linux-org-ru"));
        assert!(!sLink.contains("dt="));
    }
}
