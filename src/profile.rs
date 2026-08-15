use std::{collections::HashMap, path::Path};

#[derive(Debug, Clone)]
pub struct StThemeOption {
    pub id: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct StChoiceOption {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct StNumberOption {
    pub value: i32,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct StProfileSettings {
    pub style: String,
    pub format_mode: String,
    pub topics: i32,
    pub messages: i32,
    pub photos: bool,
    pub hide_adsense: bool,
    pub main_gallery: bool,
    pub avatar: String,
    pub tracker_mode: String,
    pub old_tracker: bool,
    pub old_notifications: bool,
    pub reaction_notification: bool,
}

pub type ThemeOption = StThemeOption;
pub type ChoiceOption = StChoiceOption;
pub type NumberOption = StNumberOption;
pub type ProfileSettings = StProfileSettings;

pub const DEFAULT_STYLE: &str = "tango-auto";
pub const DEFAULT_FORMAT_MODE: &str = "markdown";
pub const DEFAULT_TOPICS: i32 = 30;
pub const DEFAULT_MESSAGES: i32 = 50;
pub const DEFAULT_AVATAR: &str = "empty";
pub const DEFAULT_TRACKER_MODE: &str = "main";

pub const TOPICS_VALUES: &[i32] = &[30, 50, 100, 200, 300, 500];
pub const MESSAGES_VALUES: &[i32] = &[25, 50, 100, 200, 300, 500];
pub const THEMES: &[(&str, &str, bool)] = &[
    ("tango", "tango", false),
    ("tango-light", "tango-light", false),
    ("tango-auto", "tango-auto", false),
    ("black", "black", false),
    ("white2", "white2", true),
    ("waltz", "waltz", true),
    ("zomg_ponies", "zomg_ponies", true),
];
pub const AVATARS: &[&str] = &[
    "empty",
    "identicon",
    "monsterid",
    "wavatar",
    "retro",
    "robohash",
];
/// Matches TrackerFilterEnum.canBeDefault=true (ALL, MAIN) - Java doesn't
/// let a user save NOTALKS/TECH as their default tracker mode, only use
/// them as a one-off `?filter=` query param.
pub const TRACKER_MODES: &[(&str, &str)] = &[("all", "все"), ("main", "основные")];
/// Matches UserPermissionService.allowedFormats for a logged-in user
/// (Lorcode, LorcodeUlb, Markdown) - stored/form values are MarkupType's
/// `formId` ("markdown"/"lorcode"/"ntobr"). Java deliberately removed raw
/// HTML mode as a selectable format (MarkupType.Html) since it has no
/// sanitizing pass of its own - keeping it selectable here would silently
/// re-open the raw-HTML-passthrough issue that was closed in markup.rs's
/// ammonia sanitizer pass. LorcodeUlb is `deprecated=true`, gated by score
/// the same way deprecated THEMES are (see `format_options` below) -
/// unlike THEMES, its stored `BBCODE_ULB` mode is preserved so single
/// newlines render as explicit line breaks just as they do in Java.
pub const FORMAT_MODES: &[(&str, &str, bool)] = &[
    ("markdown", "Markdown", false),
    ("lorcode", "LORCODE", false),
    ("ntobr", "User line break", true),
];

impl Default for StProfileSettings {
    fn default() -> Self {
        Self {
            style: DEFAULT_STYLE.to_string(),
            format_mode: DEFAULT_FORMAT_MODE.to_string(),
            topics: DEFAULT_TOPICS,
            messages: DEFAULT_MESSAGES,
            photos: true,
            hide_adsense: true,
            main_gallery: true,
            avatar: DEFAULT_AVATAR.to_string(),
            tracker_mode: DEFAULT_TRACKER_MODE.to_string(),
            old_tracker: false,
            old_notifications: false,
            reaction_notification: true,
        }
    }
}

impl StProfileSettings {
    pub fn from_hstore_text(opt_text: Option<String>) -> Self {
        let mut settings = Self::default();
        let map = opt_text
            .as_deref()
            .map(parse_hstore_text)
            .unwrap_or_default();
        if let Some(style) = map.get("style").filter(|s| is_style(s)) {
            settings.style = style.clone();
        }
        if let Some(mode) = map.get("format.mode").filter(|s| is_format_mode(s)) {
            settings.format_mode = mode.clone();
        }
        // Not filtered against TOPICS_VALUES/MESSAGES_VALUES: a value
        // already in storage was valid (or an intentionally-preserved
        // legacy value, see `apply_form`) at the time it was saved -
        // re-validating on every read would silently discard exactly the
        // legacy values `apply_form`'s grace period exists to keep.
        if let Some(topics) = map.get("topics").and_then(|s| s.parse::<i32>().ok()) {
            settings.topics = topics;
        }
        if let Some(messages) = map.get("messages").and_then(|s| s.parse::<i32>().ok()) {
            settings.messages = messages;
        }
        if let Some(value) = map.get("photos") {
            settings.photos = parse_bool(value);
        }
        if let Some(value) = map.get("hideAdsense") {
            settings.hide_adsense = parse_bool(value);
        }
        if let Some(value) = map.get("mainGallery") {
            settings.main_gallery = parse_bool(value);
        }
        if let Some(value) = map.get("avatar").filter(|s| AVATARS.contains(&s.as_str())) {
            settings.avatar = value.clone();
        }
        if let Some(value) = map
            .get("trackerMode")
            .filter(|s| TRACKER_MODES.iter().any(|(v, _)| v == s))
        {
            settings.tracker_mode = value.clone();
        }
        if let Some(value) = map.get("oldTracker") {
            settings.old_tracker = parse_bool(value);
        }
        if let Some(value) = map.get("oldNotifications") {
            settings.old_notifications = parse_bool(value);
        }
        if let Some(value) = map.get("reactionNotification") {
            settings.reaction_notification = parse_bool(value);
        }
        settings
    }

    /// EditSettingsController.updateSettings: starts from the *current*
    /// saved profile (so anything not touched below - or rejected below -
    /// keeps its old value, never silently resets to a hardcoded default)
    /// and hard-errors (`BadInputException`) on an invalid style/format/
    /// avatar. `topics`/`messages` get a legacy-value grace: an
    /// out-of-list value is accepted if it equals the value already
    /// saved (so old custom values submitted unchanged by a stale form
    /// don't get rejected), but a *new* out-of-list value is still an
    /// error. `trackerMode` alone silently falls back to the default on
    /// an invalid value, matching `TrackerFilterEnum.getByValue(..).getOrElse(default)`.
    pub fn apply_form(&self, form: &HashMap<String, String>) -> std::result::Result<Self, String> {
        let mut settings = self.clone();

        let topics = form
            .get("topics")
            .and_then(|s| s.parse::<i32>().ok())
            .ok_or("некорректное число тем")?;
        if !TOPICS_VALUES.contains(&topics) && topics != self.topics {
            return Err("некорректное число тем".to_string());
        }
        settings.topics = topics;

        let messages = form
            .get("messages")
            .and_then(|s| s.parse::<i32>().ok())
            .ok_or("некорректное число комментариев")?;
        if !MESSAGES_VALUES.contains(&messages) && messages != self.messages {
            return Err("некорректное число комментариев".to_string());
        }
        settings.messages = messages;

        let style = form.get("style").ok_or("неправльное название темы")?;
        if !is_style(style) {
            return Err("неправльное название темы".to_string());
        }
        settings.style = style.clone();

        let format_mode = form
            .get("format_mode")
            .ok_or("некорректный режим форматирования")?;
        if !is_format_mode(format_mode) {
            return Err("некорректный режим форматирования".to_string());
        }
        settings.format_mode = format_mode.clone();

        let avatar = form.get("avatar").ok_or("invalid avatar value")?;
        if !AVATARS.contains(&avatar.as_str()) {
            return Err("invalid avatar value".to_string());
        }
        settings.avatar = avatar.clone();

        settings.photos = form.contains_key("photos");
        settings.hide_adsense = form.contains_key("hideAdsense");
        settings.main_gallery = form.contains_key("mainGallery");
        settings.old_tracker = form.contains_key("oldTracker");
        settings.old_notifications = form.contains_key("oldNotifications");
        settings.reaction_notification = form.contains_key("reactionNotification");
        settings.tracker_mode = form
            .get("trackerMode")
            .filter(|s| TRACKER_MODES.iter().any(|(v, _)| v == s))
            .cloned()
            .unwrap_or_else(|| DEFAULT_TRACKER_MODE.to_string());

        Ok(settings)
    }

    pub fn to_hstore_arrays(&self) -> (Vec<String>, Vec<String>) {
        (
            vec![
                "style".into(),
                "format.mode".into(),
                "topics".into(),
                "messages".into(),
                "photos".into(),
                "hideAdsense".into(),
                "mainGallery".into(),
                "avatar".into(),
                "trackerMode".into(),
                "oldTracker".into(),
                "oldNotifications".into(),
                "reactionNotification".into(),
            ],
            vec![
                self.style.clone(),
                self.format_mode.clone(),
                self.topics.to_string(),
                self.messages.to_string(),
                self.photos.to_string(),
                self.hide_adsense.to_string(),
                self.main_gallery.to_string(),
                self.avatar.clone(),
                self.tracker_mode.clone(),
                self.old_tracker.to_string(),
                self.old_notifications.to_string(),
                self.reaction_notification.to_string(),
            ],
        )
    }

    /// Matches EditSettingsController.showForm: users below
    /// UserPermissionService.DeprecatedFeaturesScore only see deprecated
    /// themes in the list if they're already using one (so the dropdown
    /// doesn't silently drop their current selection).
    pub fn theme_options(&self, score: i32) -> Vec<StThemeOption> {
        const DEPRECATED_FEATURES_SCORE: i32 = 500;
        THEMES
            .iter()
            .filter(|(id, _, deprecated)| {
                !*deprecated || score >= DEPRECATED_FEATURES_SCORE || self.style == *id
            })
            .map(|(id, label, _)| StThemeOption {
                id,
                label,
                selected: self.style == *id,
            })
            .collect()
    }

    pub fn topic_options(&self) -> Vec<StNumberOption> {
        let mut vecValues = TOPICS_VALUES.to_vec();
        if !vecValues.contains(&self.topics) {
            vecValues.push(self.topics);
            vecValues.sort_unstable();
        }
        vecValues
            .into_iter()
            .map(|iValue| StNumberOption {
                value: iValue,
                selected: self.topics == iValue,
            })
            .collect()
    }

    pub fn message_options(&self) -> Vec<StNumberOption> {
        let mut vecValues = MESSAGES_VALUES.to_vec();
        if !vecValues.contains(&self.messages) {
            vecValues.push(self.messages);
            vecValues.sort_unstable();
        }
        vecValues
            .into_iter()
            .map(|iValue| StNumberOption {
                value: iValue,
                selected: self.messages == iValue,
            })
            .collect()
    }

    pub fn avatar_options(&self) -> Vec<StChoiceOption> {
        AVATARS
            .iter()
            .map(|v| StChoiceOption {
                value: v,
                label: v,
                selected: self.avatar == *v,
            })
            .collect()
    }

    pub fn tracker_options(&self) -> Vec<StChoiceOption> {
        TRACKER_MODES
            .iter()
            .map(|(value, label)| StChoiceOption {
                value,
                label,
                selected: self.tracker_mode == *value,
            })
            .collect()
    }

    /// EditSettingsController.showForm: deprecated format modes (LorcodeUlb)
    /// are only offered if score>=DeprecatedFeaturesScore or already selected.
    pub fn format_options(&self, score: i32) -> Vec<StChoiceOption> {
        const DEPRECATED_FEATURES_SCORE: i32 = 500;
        FORMAT_MODES
            .iter()
            .filter(|(value, _, deprecated)| {
                !*deprecated || score >= DEPRECATED_FEATURES_SCORE || self.format_mode == *value
            })
            .map(|(value, label, _)| StChoiceOption {
                value,
                label,
                selected: self.format_mode == *value,
            })
            .collect()
    }
}

pub fn is_style(value: &str) -> bool {
    THEMES.iter().any(|(id, _, _)| *id == value)
}
pub fn is_format_mode(value: &str) -> bool {
    FORMAT_MODES.iter().any(|(id, _, _)| *id == value)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "true" | "t" | "yes" | "on" | "1")
}

/// Minimal parser for PostgreSQL hstore text output such as `"style"=>"tango-auto", "topics"=>"30"`.
/// It intentionally accepts simple unquoted legacy output as well, because old dumps differ.
fn parse_hstore_text(value: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in value.split(',') {
        let Some((key, val)) = part.split_once("=>") else {
            continue;
        };
        let key = clean_hstore_token(key);
        let val = clean_hstore_token(val);
        if !key.is_empty() {
            map.insert(key, val);
        }
    }
    map
}

fn clean_hstore_token(token: &str) -> String {
    token.trim().trim_matches('"').replace("\\\"", "\"")
}

/// `UserService.DisabledUserpic`: a 1x1 transparent placeholder used
/// whenever the viewer disabled avatars (`avatarMode=="empty"`) or the
/// target has no email to derive a Gravatar hash from.
pub const DISABLED_USERPIC: &str = "/img/p.gif";

/// Java's `Userpic` value returned by `UserService.getUserpic`.
///
/// A userpic is never an URL alone: the JSP tag writes the dimensions
/// calculated from the local image header as part of the public DOM.  Keeping
/// the three values together prevents individual routes from silently
/// reverting to a square 150x150 box for non-square uploads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUserpic {
    pub sUrl: String,
    pub iWidth: i32,
    pub iHeight: i32,
}

fn gravatar_url(email: &str, avatar_mode: &str, size: u32) -> String {
    use md5::{Digest, Md5};
    let non_exist = if avatar_mode == "empty" {
        "blank"
    } else {
        avatar_mode
    };
    let hash = Md5::digest(email.to_lowercase().as_bytes())
        .iter()
        .map(|iByte| format!("{iByte:02x}"))
        .collect::<String>();
    format!("https://secure.gravatar.com/avatar/{hash}?s={size}&r=g&d={non_exist}&f=y")
}

fn sEffectiveAvatarMode(sViewerAvatarMode: &str, bMysteryMan: bool) -> &str {
    if bMysteryMan && sViewerAvatarMode == "empty" {
        "mm"
    } else {
        sViewerAvatarMode
    }
}

fn stDisabledUserpic() -> StUserpic {
    StUserpic {
        sUrl: DISABLED_USERPIC.to_owned(),
        iWidth: 1,
        iHeight: 1,
    }
}

fn optJavaUserpicFormat(sPhoto: &str) -> Option<image::ImageFormat> {
    match Path::new(sPhoto)
        .extension()
        .and_then(|sExtension| sExtension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => Some(image::ImageFormat::Jpeg),
        Some("png") => Some(image::ImageFormat::Png),
        Some("gif") => Some(image::ImageFormat::Gif),
        _ => None,
    }
}

fn stJavaUserpicDimensions(
    pathPhoto: &Path,
    stFormat: image::ImageFormat,
) -> image::ImageResult<(u32, u32)> {
    let stFile = std::fs::File::open(pathPhoto).map_err(image::ImageError::IoError)?;
    image::ImageReader::with_format(std::io::BufReader::new(stFile), stFormat).into_dimensions()
}

/// `ImageInfo.scale(150)` from the original application.
pub fn stScaleUserpicDimensions(iWidth: u32, iHeight: u32) -> (i32, i32) {
    if iWidth <= 150 && iHeight <= 150 {
        return (iWidth as i32, iHeight as i32);
    }
    if iWidth > iHeight {
        (150, (u64::from(iHeight) * 150 / u64::from(iWidth)) as i32)
    } else {
        ((u64::from(iWidth) * 150 / u64::from(iHeight)) as i32, 150)
    }
}

/// Filesystem-aware equivalent of Java `UserService.getUserpic`.
///
/// Local photos take precedence over Gravatar, but only when the referenced
/// file exists and has a readable PNG/JPEG/GIF header.  Missing or corrupt
/// migration media therefore follows the same configured Gravatar/disabled
/// fallback as Java instead of emitting a permanently broken `/photos/*`
/// element. `sUploadRoot` is `SiteConfig.getUploadPath`, not the `photos`
/// directory itself.
pub fn stResolveUserpic(
    sUploadRoot: &Path,
    sViewerAvatarMode: &str,
    bMysteryMan: bool,
    bAnonymous: bool,
    optLocalPhoto: Option<&str>,
    optEmail: Option<&str>,
) -> StUserpic {
    let sAvatarMode = sEffectiveAvatarMode(sViewerAvatarMode, bMysteryMan);

    if bAnonymous && bMysteryMan {
        return StUserpic {
            sUrl: gravatar_url("anonymous@linux.org.ru", sAvatarMode, 150),
            iWidth: 150,
            iHeight: 150,
        };
    }

    if let Some(sPhoto) = optLocalPhoto.filter(|sPhoto| !sPhoto.is_empty()) {
        let bFilename = Path::new(sPhoto)
            .file_name()
            .and_then(|sFilename| sFilename.to_str())
            == Some(sPhoto);
        if let (true, Some(stFormat)) = (bFilename, optJavaUserpicFormat(sPhoto)) {
            match stJavaUserpicDimensions(&sUploadRoot.join("photos").join(sPhoto), stFormat) {
                Ok((iWidth, iHeight)) if iWidth > 0 && iHeight > 0 => {
                    let (iWidth, iHeight) = stScaleUserpicDimensions(iWidth, iHeight);
                    return StUserpic {
                        sUrl: format!("/photos/{sPhoto}"),
                        iWidth,
                        iHeight,
                    };
                }
                Ok((iWidth, iHeight)) => {
                    tracing::warn!(
                        userpic = sPhoto,
                        width = iWidth,
                        height = iHeight,
                        "local userpic dimensions are invalid"
                    );
                }
                Err(stError) => {
                    tracing::warn!(
                        userpic = sPhoto,
                        error = %stError,
                        "local userpic is unavailable or invalid"
                    );
                }
            }
        } else {
            tracing::warn!(userpic = sPhoto, "local userpic filename is invalid");
        }
    }

    // `User.hasEmail` checks nullness only.  An empty-but-present legacy DB
    // value is therefore hashed exactly like any other address.
    let Some(sEmail) = optEmail else {
        return stDisabledUserpic();
    };
    if sAvatarMode == "empty" {
        return stDisabledUserpic();
    }
    StUserpic {
        sUrl: gravatar_url(sEmail, sAvatarMode, 150),
        iWidth: 150,
        iHeight: 150,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_notifications_round_trips_the_java_profile_key() {
        let stSettings = ProfileSettings::from_hstore_text(Some(
            r#""oldNotifications"=>"true", "reactionNotification"=>"false""#.into(),
        ));
        assert!(stSettings.old_notifications);
        assert!(!stSettings.reaction_notification);

        let (vecKeys, vecValues) = stSettings.to_hstore_arrays();
        let iIndex = vecKeys
            .iter()
            .position(|sKey| sKey == "oldNotifications")
            .unwrap();
        assert_eq!(vecValues[iIndex], "true");
    }

    #[test]
    fn settings_keep_and_sort_legacy_page_sizes_like_java() {
        let stSettings = StProfileSettings {
            topics: 75,
            messages: 40,
            ..StProfileSettings::default()
        };

        let vecTopics = stSettings.topic_options();
        assert_eq!(
            vecTopics
                .iter()
                .map(|stValue| stValue.value)
                .collect::<Vec<_>>(),
            vec![30, 50, 75, 100, 200, 300, 500]
        );
        assert!(
            vecTopics
                .iter()
                .any(|stValue| stValue.value == 75 && stValue.selected)
        );

        let vecMessages = stSettings.message_options();
        assert_eq!(
            vecMessages
                .iter()
                .map(|stValue| stValue.value)
                .collect::<Vec<_>>(),
            vec![25, 40, 50, 100, 200, 300, 500]
        );
        assert!(
            vecMessages
                .iter()
                .any(|stValue| stValue.value == 40 && stValue.selected)
        );
    }

    #[test]
    fn local_userpic_is_header_checked_and_scaled_like_java() {
        let iNonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-profile-userpic-{}-{iNonce}",
            std::process::id()
        ));
        let pathPhotos = pathRoot.join("photos");
        std::fs::create_dir_all(&pathPhotos).unwrap();
        image::RgbImage::from_pixel(300, 150, image::Rgb([20, 40, 60]))
            .save(pathPhotos.join("42.png"))
            .unwrap();

        let stUserpic = stResolveUserpic(
            &pathRoot,
            "empty",
            false,
            false,
            Some("42.png"),
            Some("user@example.test"),
        );
        assert_eq!(stUserpic.sUrl, "/photos/42.png");
        assert_eq!((stUserpic.iWidth, stUserpic.iHeight), (150, 75));

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[test]
    fn local_userpic_format_must_match_its_java_extension() {
        let iNonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-profile-userpic-format-{}-{iNonce}",
            std::process::id()
        ));
        let pathPhotos = pathRoot.join("photos");
        std::fs::create_dir_all(&pathPhotos).unwrap();
        image::RgbImage::from_pixel(150, 100, image::Rgb([20, 40, 60]))
            .save_with_format(pathPhotos.join("42.png"), image::ImageFormat::Jpeg)
            .unwrap();

        let stUserpic = stResolveUserpic(&pathRoot, "empty", false, false, Some("42.png"), None);
        assert_eq!(stUserpic, stDisabledUserpic());

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[test]
    fn unavailable_local_userpic_uses_java_fallback_contract() {
        let iNonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-missing-userpic-root-{}-{iNonce}",
            std::process::id()
        ));

        assert_eq!(
            stResolveUserpic(
                &pathRoot,
                "empty",
                false,
                false,
                Some("42.png"),
                Some("user@example.test"),
            ),
            StUserpic {
                sUrl: DISABLED_USERPIC.to_owned(),
                iWidth: 1,
                iHeight: 1,
            }
        );

        let stGravatar = stResolveUserpic(
            &pathRoot,
            "identicon",
            false,
            false,
            Some("42.png"),
            Some("User@Example.Test"),
        );
        assert_eq!(
            stGravatar.sUrl,
            "https://secure.gravatar.com/avatar/08ea5c18206e8006a5f3ca67e5d844dd?s=150&r=g&d=identicon&f=y"
        );
        assert_eq!((stGravatar.iWidth, stGravatar.iHeight), (150, 150));

        let stEmptyEmail = stResolveUserpic(&pathRoot, "identicon", false, false, None, Some(""));
        assert_eq!(
            stEmptyEmail.sUrl,
            "https://secure.gravatar.com/avatar/d41d8cd98f00b204e9800998ecf8427e?s=150&r=g&d=identicon&f=y"
        );
        assert_ne!(stEmptyEmail.sUrl, DISABLED_USERPIC);
        assert!(
            gravatar_url(" User@Example.Test ", "identicon", 150)
                .contains("/6c5bf0d04da6a4ad142847d064264884?")
        );

        for sInvalidPhoto in ["../42.png", "https://example.test/42.png", "42.webp"] {
            let stInvalid = stResolveUserpic(
                &pathRoot,
                "identicon",
                false,
                false,
                Some(sInvalidPhoto),
                Some("user@example.test"),
            );
            assert!(
                stInvalid.sUrl.ends_with("?s=150&r=g&d=identicon&f=y"),
                "invalid local photo must use the configured fallback: {sInvalidPhoto}"
            );
        }

        let stMissingTopicPhoto = stResolveUserpic(
            &pathRoot,
            "empty",
            true,
            false,
            Some("42.png"),
            Some("user@example.test"),
        );
        assert!(stMissingTopicPhoto.sUrl.ends_with("?s=150&r=g&d=mm&f=y"));
        assert_eq!(
            (stMissingTopicPhoto.iWidth, stMissingTopicPhoto.iHeight),
            (150, 150)
        );
    }

    #[test]
    fn anonymous_mystery_man_uses_mm_even_when_empty_is_selected() {
        let stUserpic = stResolveUserpic(Path::new("/not-used"), "empty", true, true, None, None);
        assert!(stUserpic.sUrl.ends_with("?s=150&r=g&d=mm&f=y"));
        assert_eq!((stUserpic.iWidth, stUserpic.iHeight), (150, 150));
    }

    #[test]
    fn anonymous_topic_profile_and_comment_fallbacks_match_java() {
        let iNonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-profile-anonymous-userpic-{}-{iNonce}",
            std::process::id()
        ));
        let pathPhotos = pathRoot.join("photos");
        std::fs::create_dir_all(&pathPhotos).unwrap();
        image::RgbImage::from_pixel(300, 150, image::Rgb([20, 40, 60]))
            .save(pathPhotos.join("2.png"))
            .unwrap();

        // TopicPrepareService and WhoisController use mysteryMan=true, so
        // anonymous always gets the fixed Gravatar even if a local filename
        // exists on the anonymous DB row.
        let stTopic = stResolveUserpic(
            &pathRoot,
            "empty",
            true,
            true,
            Some("2.png"),
            Some("anonymous@example.test"),
        );
        assert!(stTopic.sUrl.ends_with("?s=150&r=g&d=mm&f=y"));
        assert_eq!((stTopic.iWidth, stTopic.iHeight), (150, 150));

        // CommentPrepareService uses mysteryMan=false and follows the normal
        // local-photo/fallback branch for the same account.
        let stComment = stResolveUserpic(
            &pathRoot,
            "empty",
            false,
            true,
            Some("2.png"),
            Some("anonymous@example.test"),
        );
        assert_eq!(stComment.sUrl, "/photos/2.png");
        assert_eq!((stComment.iWidth, stComment.iHeight), (150, 75));
        assert_eq!(
            stResolveUserpic(&pathRoot, "empty", false, true, None, None),
            stDisabledUserpic()
        );

        std::fs::remove_dir_all(pathRoot).unwrap();
    }
}
