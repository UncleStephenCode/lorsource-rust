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
