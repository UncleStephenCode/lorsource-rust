#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StTagCloudRow {
    pub sValue: String,
    pub iCounter: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTagCloudItem {
    pub sValue: String,
    pub iWeight: i32,
    pub sUrl: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StGalleryBoxletRow {
    pub iMsgId: i32,
    pub iUserId: i32,
    pub sTitle: String,
    pub iStat: i32,
    pub sGroupUrlName: String,
    pub iImageId: i32,
    pub sExtension: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StGalleryBoxletItem {
    pub sTitle: String,
    pub sAltTitle: String,
    pub iStat: i32,
    pub sUserNick: String,
    pub sLink: String,
    pub sImageMedium: String,
    pub sImageSrcset: String,
    pub iImageWidth: u32,
    pub iImageHeight: u32,
    pub sImagePaddingPercent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StTopicBoxletRow {
    pub iMsgId: i32,
    pub sGroupUrlName: String,
    pub iSectionId: i32,
    pub sTitle: String,
    pub dtLastModified: chrono::DateTime<chrono::Utc>,
    pub iCommentCount: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicBoxletItem {
    pub sMessageUrl: String,
    pub sTitle: String,
    pub iCommentCount: i32,
    pub iPages: i32,
    pub optLastPageUrl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StPollBoxletRow {
    pub iPollId: i32,
    pub iTopicId: i32,
    pub bMultiSelect: bool,
    pub sTitle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StPollVariantResult {
    pub iId: i32,
    pub sLabel: String,
    pub iVotes: i32,
    pub bUserVoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StPollBoxlet {
    pub iPollId: i32,
    pub iTopicId: i32,
    pub bMultiSelect: bool,
    pub sTitle: String,
    pub vecVariants: Vec<StPollVariantResult>,
    pub iVotes: i32,
    pub iUsers: i32,
    pub bUserVoted: bool,
}
