//! Resolves the saved Java-compatible profile style and applies the exact
//! stylesheet/data-theme mapping used by `WEB-INF/jsp/head.jsp`.

use crate::{profile::is_style, request_timezone, state::AppState};
use axum::{
    body::Body,
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;

const DEFAULT_STYLE: &str = "tango-auto";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThemeView<'a> {
    style: &'a str,
    color_mode: Option<&'a str>,
    stylesheet: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StThemeProfile {
    sStyle: String,
    optNick: Option<String>,
    sFormatMode: String,
    iUnreadEvents: i32,
}

/// Trusted identity selected by an authenticated handler for theming its own
/// response.  This is needed when the handler has just changed credentials:
/// the request still carries the old password-bound remember-me cookie, while
/// the response already carries the refreshed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StResponseThemeUser {
    iUserId: i32,
}

pub(crate) fn vUseAuthenticatedThemeForResponse(stResponse: &mut Response, iUserId: i32) {
    stResponse
        .extensions_mut()
        .insert(StResponseThemeUser { iUserId });
}

#[cfg(test)]
pub(crate) fn optResponseThemeUserId(stResponse: &Response) -> Option<i32> {
    stResponse
        .extensions()
        .get::<StResponseThemeUser>()
        .map(|stIdentity| stIdentity.iUserId)
}

fn theme_view(style: &str) -> ThemeView<'_> {
    match style {
        "tango" => ThemeView {
            style,
            color_mode: Some("dark"),
            stylesheet: "/tango/combined.css",
        },
        "tango-light" => ThemeView {
            style,
            color_mode: Some("light"),
            stylesheet: "/tango/combined.css",
        },
        "tango-auto" => ThemeView {
            style,
            color_mode: Some("auto"),
            stylesheet: "/tango/combined.css",
        },
        "black" => ThemeView {
            style,
            color_mode: None,
            stylesheet: "/black/combined.css",
        },
        "white2" => ThemeView {
            style,
            color_mode: None,
            stylesheet: "/white2/combined.css",
        },
        "waltz" => ThemeView {
            style,
            color_mode: None,
            stylesheet: "/waltz/combined.css",
        },
        "zomg_ponies" => ThemeView {
            style,
            color_mode: None,
            stylesheet: "/zomg_ponies/combined.css",
        },
        _ => theme_view(DEFAULT_STYLE),
    }
}

async fn stResolveProfile(
    state: &AppState,
    jar: &CookieJar,
    optResponseUserId: Option<i32>,
) -> StThemeProfile {
    let optUserId = match optResponseUserId {
        Some(iUserId) => Some(iUserId),
        None => crate::auth::optUserIdFromCookies(&state.pool, jar, &state.config.site_secret)
            .await
            .ok()
            .flatten(),
    };
    if let Some(user_id) = optUserId {
        let optProfile: Option<(Option<String>, Option<String>, String, i32)> = sqlx::query_as(
            "SELECT us.settings->'style', us.settings->'format.mode', u.nick, \
                    COALESCE(u.unread_events,0) \
             FROM users u LEFT JOIN user_settings us ON us.id=u.id
             WHERE u.id=$1 AND u.activated AND NOT COALESCE(u.blocked,false)",
        )
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        if let Some((optStyle, optFormatMode, sNick, iUnreadEvents)) = optProfile {
            return StThemeProfile {
                sStyle: optStyle
                    .filter(|value| is_style(value))
                    .unwrap_or_else(|| DEFAULT_STYLE.to_string()),
                optNick: Some(sNick),
                sFormatMode: optFormatMode
                    .filter(|value| crate::profile::is_format_mode(value))
                    .unwrap_or_else(|| crate::profile::DEFAULT_FORMAT_MODE.to_string()),
                iUnreadEvents,
            };
        }
    }
    StThemeProfile {
        sStyle: DEFAULT_STYLE.to_string(),
        optNick: None,
        sFormatMode: crate::profile::DEFAULT_FORMAT_MODE.to_string(),
        iUnreadEvents: 0,
    }
}

fn login_block(nick: Option<&str>, pony: bool, regmenu_head_class: bool) -> String {
    match nick {
        Some(nick) => {
            let greeting = if pony {
                "дружбомагии тебе"
            } else {
                "добро пожаловать"
            };
            format!(
                "{greeting}, <a style=\"text-decoration: none\" href=\"/people/{}/profile\">{}</a>",
                urlencoding::encode(nick),
                html_escape::encode_text(nick),
            )
        }
        None => format!(
            "<div id=\"regmenu\"{}><a href=\"/register.jsp\">Регистрация</a> - <a id=\"loginbutton\" href=\"/login.jsp\">Вход</a></div>",
            if regmenu_head_class {
                " class=\"head\""
            } else {
                ""
            }
        ),
    }
}

fn sLegacyEvents(authenticated: bool, unread_events: i32, disable_event_header: bool) -> String {
    if disable_event_header {
        return r#"<a href="notifications">Уведомления</a>"#.to_string();
    }
    if !authenticated {
        return String::new();
    }
    let sCount = if unread_events > 0 {
        format!("({unread_events})")
    } else {
        String::new()
    };
    format!(
        r#"<a href="notifications">Уведомления <span id="main_events_count">{sCount}</span></a>"#
    )
}

fn modern_header(nick: Option<&str>, unread_events: i32, disable_event_header: bool) -> String {
    let top_profile = nick.map_or_else(String::new, |nick| {
        let sEvents = if disable_event_header {
            String::new()
        } else if unread_events > 0 {
            format!(
                r#"<a href="/notifications"> <i class="icon-bell"></i><span id="main_events_count_number" class="set">{unread_events}</span></a>"#
            )
        } else {
            r#"<a href="/notifications"> <i class="icon-bell"></i><span id="main_events_count_number"></span></a>"#.to_string()
        };
        format!(
            "{sEvents}<a title=\"{}\" href=\"/people/{}/profile\"><i class=\"icon-user-circle-o\"></i></a>",
            html_escape::encode_double_quoted_attribute(nick),
            urlencoding::encode(nick),
        )
    });
    let guest = if nick.is_none() {
        login_block(None, false, true)
    } else {
        String::new()
    };
    format!(
        r#"<header id="hd"><div id="topProfile">{top_profile}</div><span id="sitetitle"><a href="/">LINUX.ORG.RU</a></span><nav class="menu"><div id="loginGreating">{guest}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/polls/">Опросы</a></li> <li><a href="/tracker/">Трекер</a></li> <li><a href="/search.jsp">Поиск</a></li></ul></nav></header><div style="clear: both"></div>"#,
    )
}

fn waltz_header(nick: Option<&str>, unread_events: i32, disable_event_header: bool) -> String {
    let top_profile = nick.map_or_else(String::new, |nick| {
        format!(
            r#"<a style="text-decoration: none" href="/people/{}/profile">{}</a>"#,
            urlencoding::encode(nick),
            html_escape::encode_text(nick)
        )
    });
    let guest = if nick.is_none() {
        login_block(None, false, true)
    } else {
        String::new()
    };
    let sEvents = sLegacyEvents(nick.is_some(), unread_events, disable_event_header);
    let events = if sEvents.is_empty() {
        String::new()
    } else {
        format!("<li>{sEvents}</li>")
    };
    format!(
        r#"<header id="hd"><div id="topProfile">{top_profile}</div><span id="sitetitle"><a href="/">LINUX.ORG.RU</a></span><nav class="menu"><div id="loginGreating">{guest}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></nav></header><div style="clear: both"></div>"#
    )
}

fn black_header(
    nick: Option<&str>,
    unread_events: i32,
    disable_event_header: bool,
    main_page: bool,
) -> String {
    let login = login_block(nick, false, main_page);
    let sEvents = sLegacyEvents(nick.is_some(), unread_events, disable_event_header);
    if !main_page {
        let events = if sEvents.is_empty() {
            String::new()
        } else {
            format!("{sEvents} - ")
        };
        return format!(
            r#"<table border="0" cellspacing="0" cellpadding="0" width="100%" class="head"><tr><td rowspan="2" align="left"><a href="/"><img src="/black/lor-new.png" width="282" height="60" alt="Linux.org.ru"></a></td><td align="right">{login}</td></tr><tr><td align="right" valign="bottom"><a style="text-decoration:none" href="/news/">Новости</a> - <a style="text-decoration:none" href="/gallery/">Галерея</a> - <a style="text-decoration:none" href="/articles/">Статьи</a> - <a style="text-decoration:none" href="/forum/">Форум</a> - {events}<a style="text-decoration:none" href="/tracker/">Трекер</a> - <a style="text-decoration:none" href="/search.jsp">Поиск</a></td></tr></table>"#,
        );
    }
    format!(
        r#"<a href="/"><img style="float:left;border:0" src="/black/lorlogo-try.png" alt="Русская информация об ОС LINUX" width="270" height="208"></a><div id="hd"><div id="head-main"><table><tr><td><a href="/news/">Новости</a></td><td><a href="/tracker/">Трекер</a></td><td><a href="/about">О сервере</a></td></tr><tr><td><a href="/gallery/">Галерея</a></td><td><a href="/forum/">Форум</a></td><td>{sEvents}</td></tr><tr><td><a href="/articles/">Статьи</a></td><td></td><td><a href="/search.jsp">Поиск</a></td></tr></table><br></div><div style="right:5px;text-align:right;top:5px;position:absolute" class="head">{login}</div></div>"#,
    )
}

fn white2_header(nick: Option<&str>, unread_events: i32, disable_event_header: bool) -> String {
    let login = nick.map_or_else(
        || login_block(None, false, true),
        |nick| format!(r#"добро пожаловать,&nbsp;<a style="text-decoration: none" href="/people/{}/profile">{}</a>"#, urlencoding::encode(nick), html_escape::encode_text(nick)),
    );
    let sEvents = sLegacyEvents(nick.is_some(), unread_events, disable_event_header);
    let events = if sEvents.is_empty() {
        String::new()
    } else {
        format!("<li>{sEvents}</li>")
    };
    format!(
        r#"<div id="hd"><div id="hdtux"><img src="/img/Tux.svg" height="100%" alt="Linux"></div><a id="sitetitle" href="/">LINUX.ORG.RU</a><ul class="menu"><li id="loginGreating">{login}</li> <li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></div><div style="clear: both"></div>"#,
    )
}

fn pony_header(nick: Option<&str>, unread_events: i32, disable_event_header: bool) -> String {
    let login = login_block(nick, true, true);
    let sEvents = sLegacyEvents(nick.is_some(), unread_events, disable_event_header);
    let events = if sEvents.is_empty() {
        String::new()
    } else {
        format!("<li>{sEvents}</li>")
    };
    format!(
        r#"<div id="hd"><a id="sitetitle" href="/"><img src="/zomg_ponies/img/twilight_logo.png" id="twilight_logo" alt="">PONY.ORG.RU</a><div class="menu"><div id="loginGreating">{login}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></div></div><div style="clear: both"></div>"#,
    )
}

fn render_header(
    style: &str,
    nick: Option<&str>,
    unread_events: i32,
    disable_event_header: bool,
    main_page: bool,
) -> String {
    match style {
        "black" => black_header(nick, unread_events, disable_event_header, main_page),
        "white2" => white2_header(nick, unread_events, disable_event_header),
        "waltz" => waltz_header(nick, unread_events, disable_event_header),
        "zomg_ponies" => pony_header(nick, unread_events, disable_event_header),
        _ => modern_header(nick, unread_events, disable_event_header),
    }
}

const THEME_INDICATOR: &str = r#"<div id="theme-indicator"><span class="theme-indicator-dark" title="Темная тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="M9.37,5.51C9.19,6.15,9.1,6.82,9.1,7.5c0,4.08,3.32,7.4,7.4,7.4c0.68,0,1.35-0.09,1.99-0.27C17.45,17.19,14.93,19,12,19 c-3.86,0-7-3.14-7-7C5,9.07,6.81,6.55,9.37,5.51z M12,3c-4.97,0-9,4.03-9,9s4.03,9,9,9s9-4.03,9-9c0-0.46-0.04-0.92-0.1-1.36 c-0.98,1.37-2.58,2.26-4.4,2.26c-2.98,0-5.4-2.42-5.4-5.4c0-1.81,0.89-3.42,2.26-4.4C12.92,3.04,12.46,3,12,3L12,3z"></path></svg></span><span class="theme-indicator-light" title="Светлая тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="M12,9c1.65,0,3,1.35,3,3s-1.35,3-3,3s-3-1.35-3-3S10.35,9,12,9 M12,7c-2.76,0-5,2.24-5,5s2.24,5,5,5s5-2.24,5-5 S14.76,7,12,7L12,7z M2,13l2,0c0.55,0,1-0.45,1-1s-0.45-1-1-1l-2,0c-0.55,0-1,0.45-1,1S1.45,13,2,13z M20,13l2,0c0.55,0,1-0.45,1-1 s-0.45-1-1-1l-2,0c-0.55,0-1,0.45-1,1S19.45,13,20,13z M11,2v2c0,0.55,0.45,1,1,1s1-0.45,1-1V2c0-0.55-0.45-1-1-1S11,1.45,11,2z M11,20v2c0,0.55,0.45,1,1,1s1-0.45,1-1v-2c0-0.55-0.45-1-1-1C11.45,19,11,19.45,11,20z M5.99,4.58c-0.39-0.39-1.03-0.39-1.41,0 c-0.39,0.39-0.39,1.03,0,1.41l1.06,1.06c0.39,0.39,1.03,0.39,1.41,0s0.39-1.03,0-1.41L5.99,4.58z M18.36,16.95 c-0.39-0.39-1.03-0.39-1.41,0c-0.39,0.39-0.39,1.03,0,1.41l1.06,1.06c0.39,0.39,1.03,0.39,1.41,0c0.39-0.39,0.39-1.03,0-1.41 L18.36,16.95z M19.42,5.99c0.39-0.39,0.39-1.03,0-1.41c-0.39,0.39-1.03-0.39-1.41,0l-1.06,1.06c-0.39,0.39-0.39,1.03,0,1.41 s1.03,0.39,1.41,0L19.42,5.99z M7.05,18.36c0.39-0.39,0.39-1.03,0-1.41c-0.39-0.39-1.03-0.39-1.41,0l-1.06,1.06 c-0.39,0.39-0.39,1.03,0,1.41s1.03,0.39,1.41,0L7.05,18.36z"></path></svg></span><span class="theme-indicator-auto" title="Системная тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="m12 21c4.971 0 9-4.029 9-9s-4.029-9-9-9-9 4.029-9 9 4.029 9 9 9zm4.95-13.95c1.313 1.313 2.05 3.093 2.05 4.95s-0.738 3.637-2.05 4.95c-1.313 1.313-3.093 2.05-4.95 2.05v-14c1.857 0 3.637 0.737 4.95 2.05z"></path></svg></span></div>"#;

fn render_footer(
    public_url: &str,
    ws_url: &str,
    authenticated: bool,
    main_page: bool,
    format_mode: &str,
) -> String {
    let info = if main_page {
        r#"<p id="ft-info"><a href="/about">О Сервере</a> - <a href="/help/rules.md">Правила форума</a><br>Разработка и&nbsp;поддержка&nbsp;— <a href="/people/maxcom/profile">Максим Валянский</a> 1998–2026<br>Сервер для сайта предоставлен «<a href="http://www.ittelo.ru/" target="_blank">ITTelo</a>»<br>Размещение сервера и&nbsp;подключение к&nbsp;сети Интернет осуществляется компанией «<a href="https://selectel.ru/?ref_code=3dce4333ba" target="_blank">Selectel</a>».</p>"#.to_string()
    } else {
        let url = html_escape::encode_double_quoted_attribute(public_url);
        let markup_help = match format_mode {
            "lorcode" => r#" - <a href="/help/lorcode.md">Разметка LORCODE</a>"#,
            "markdown" => r#" - <a href="/help/markdown.md">Разметка Markdown</a>"#,
            _ => "",
        };
        format!(
            r#"<p id="ft-info"><a href="/about">О Сервере</a> - <a href="/help/rules.md">Правила форума</a>{markup_help}<br><a href="https://github.com/maxcom/lorsource/issues">Сообщить об ошибке</a><br><a href="{url}">{url}</a></p>"#
        )
    };
    let theme_indicator = THEME_INDICATOR.replace(
        "c-0.39,0.39-1.03-0.39-1.41,0",
        "c-0.39-0.39-1.03-0.39-1.41,0",
    );
    let realtime_body = if authenticated {
        let sWsUrl = serde_json::to_string(ws_url).expect("serializing a string cannot fail");
        format!(r#"$script.ready('realtime', function() {{ RealtimeContext.start({sWsUrl}); }});"#)
    } else {
        String::new()
    };
    format!(
        r#"<footer id="ft">{info}<script type="text/javascript">{realtime_body}</script>{theme_indicator}</footer>"#
    )
}

pub async fn apply_theme(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let main_page = matches!(req.uri().path(), "/" | "/index.jsp");
    let bNotificationsPage = req.uri().path() == "/notifications";
    let sCurrentUrl = req
        .uri()
        .path_and_query()
        .map_or("/", axum::http::uri::PathAndQuery::as_str)
        .to_owned();
    let response = next.run(req).await;
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    if !is_html {
        return response;
    }

    let optResponseUserId = response
        .extensions()
        .get::<StResponseThemeUser>()
        .map(|stIdentity| stIdentity.iUserId);
    let stProfile = stResolveProfile(&state, &jar, optResponseUserId).await;
    let view = theme_view(&stProfile.sStyle);
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let Ok(mut text) = String::from_utf8(bytes.to_vec()) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let html_attributes = match view.color_mode {
        Some(mode) => format!("data-style=\"{}\" data-theme=\"{mode}\"", view.style),
        None => format!("data-style=\"{}\"", view.style),
    };
    text = text.replacen(
        "data-style=\"tango-auto\" data-theme=\"auto\"",
        &html_attributes,
        1,
    );
    text = text.replacen(
        "/tango/combined.css\" data-lor-theme-stylesheet",
        &format!("{}\" data-lor-theme-stylesheet", view.stylesheet),
        1,
    );
    let sBaseUrl = format!("{}/", state.config.public_url.trim_end_matches('/'));
    text = text.replacen(
        "<!-- LOR_BASE_URL -->",
        &html_escape::encode_double_quoted_attribute(&sBaseUrl),
        1,
    );
    let bDisableEventHeader = bNotificationsPage && stProfile.optNick.is_some();
    let mut sHeader = render_header(
        &stProfile.sStyle,
        stProfile.optNick.as_deref(),
        stProfile.iUnreadEvents,
        bDisableEventHeader,
        main_page,
    );
    if stProfile.optNick.is_none() {
        sHeader = sHeader.replacen(
            "href=\"/login.jsp\"",
            &format!(
                "href=\"/login.jsp?from={}\"",
                urlencoding::encode(&sCurrentUrl)
            ),
            1,
        );
    }
    text = text.replacen("<!-- LOR_THEME_HEADER -->", &sHeader, 1);
    text = text.replacen(
        "<!-- LOR_THEME_FOOTER -->",
        &render_footer(
            &state.config.public_url,
            &state.config.ws_url,
            stProfile.optNick.is_some(),
            main_page,
            &stProfile.sFormatMode,
        ),
        1,
    );
    let stTimezone = request_timezone::stRequestTimezone(&jar);
    text = text.replacen("<!-- LOR_TIMEZONE -->", stTimezone.name(), 1);
    text = request_timezone::sRewriteHtmlTimes(&text, stTimezone, chrono::Utc::now());
    if stProfile.sStyle == "black" && main_page {
        text = text.replacen("<body>", "<body style=\"margin-top: 0\">", 1);
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_theme_mapping_is_exact() {
        assert_eq!(theme_view("tango").color_mode, Some("dark"));
        assert_eq!(theme_view("tango-light").color_mode, Some("light"));
        assert_eq!(theme_view("tango-auto").color_mode, Some("auto"));
        for style in ["black", "white2", "waltz", "zomg_ponies"] {
            let view = theme_view(style);
            assert_eq!(view.style, style);
            assert_eq!(view.color_mode, None);
            assert_eq!(view.stylesheet, format!("/{style}/combined.css"));
        }
    }

    #[test]
    fn every_theme_bundle_is_built() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for style in ["tango", "black", "white2", "waltz", "zomg_ponies"] {
            let path = root.join("static").join(style).join("combined.css");
            let css = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                css.len() > 55_000,
                "{} is not an original compiled theme bundle",
                path.display()
            );
            assert!(
                css.contains(".swiffy-slider"),
                "{} misses Swiffy Slider",
                path.display()
            );
            assert!(
                css.contains(".tippy-box"),
                "{} misses Tippy",
                path.display()
            );
            assert!(
                css.contains(".hljs"),
                "{} misses syntax highlighting",
                path.display()
            );
            // Swiffy's upstream minified file intentionally keeps its own
            // sourceMappingURL comment, exactly as in the Java aggregation.
        }
    }

    #[test]
    fn one_theme_header_and_footer_are_selected_server_side() {
        let base = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/base.html"));
        assert_eq!(base.matches("LOR_THEME_HEADER").count(), 1);
        assert_eq!(base.matches("LOR_THEME_FOOTER").count(), 1);
        assert!(!base.contains("theme-shell.css"));
        assert_eq!(
            black_header(None, 0, false, false)
                .matches("lor-new.png")
                .count(),
            1
        );
        assert_eq!(
            black_header(None, 0, false, true)
                .matches("lorlogo-try.png")
                .count(),
            1
        );
        assert!(!black_header(None, 0, false, true).contains("Уведомления"));
        assert!(black_header(Some("user"), 0, false, true).contains("Уведомления"));
        assert!(
            !render_footer(
                "https://example/",
                "wss://example/",
                false,
                false,
                "markdown"
            )
            .contains("RealtimeContext.start")
        );
        assert!(
            render_footer(
                "https://example/",
                "wss://example/",
                false,
                false,
                "markdown"
            )
            .contains("<script type=\"text/javascript\"></script>")
        );
        let authenticated_footer = render_footer(
            "https://example/",
            "wss://example/ws-root/",
            true,
            false,
            "lorcode",
        );
        assert!(authenticated_footer.contains("$script.ready('realtime'"));
        assert!(authenticated_footer.contains("RealtimeContext.start(\"wss://example/ws-root/\")"));
        assert!(authenticated_footer.contains("Разметка LORCODE"));
    }

    #[test]
    fn inline_menu_items_keep_the_jsp_whitespace_gap() {
        for header in [
            modern_header(None, 0, false),
            waltz_header(None, 0, false),
            white2_header(None, 0, false),
            pony_header(None, 0, false),
        ] {
            assert!(header.contains("</li> <li>"));
            assert!(!header.contains("</li><li>"));
        }
    }

    #[test]
    fn profile_style_and_tango_color_mode_are_not_mixed() {
        let base = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/base.html"));
        let settings = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/settings.html"
        ));
        assert!(base.contains("theme === 'dark' || theme === 'light' || theme === 'auto'"));
        assert!(!base.contains("dataset.style.startsWith('tango')"));
        assert!(settings.contains("localStorage.removeItem('lor-theme')"));
        assert!(!settings.contains("localStorage.setItem('lor-theme'"));
        assert!(!settings.contains("lor_theme="));
    }

    #[test]
    fn unread_event_header_matches_java_theme_variants() {
        let sModern = modern_header(Some("user"), 3, false);
        assert!(sModern.contains("main_events_count_number\" class=\"set\">3"));
        assert!(sModern.contains("/people/user/profile"));

        let sModernNotifications = modern_header(Some("user"), 3, true);
        assert!(!sModernNotifications.contains("icon-bell"));
        assert!(sModernNotifications.contains("icon-user-circle-o"));

        let sLegacy = waltz_header(Some("user"), 3, false);
        assert!(sLegacy.contains("main_events_count\">(3)</span>"));
        let sLegacyNotifications = waltz_header(Some("user"), 3, true);
        assert!(sLegacyNotifications.contains("<a href=\"notifications\">Уведомления</a>"));
        assert!(!sLegacyNotifications.contains("main_events_count"));
    }

    #[test]
    fn settings_theme_controls_keep_original_dom_contract() {
        let settings = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/settings.html"
        ));
        for (name, label) in [
            ("style", "style-label"),
            ("topics", "topics-label"),
            ("messages", "messages-label"),
            ("trackerMode", "trackerMode-label"),
            ("avatar", "avatar-label"),
            ("format_mode", "format-mode-label"),
        ] {
            assert!(settings.contains(&format!("aria-labelledby=\"{label}\"")));
            assert!(settings.contains(&format!("name=\"{name}\"")));
            assert!(settings.contains(&format!("id=\"{name}-{{{{ loop.index0 }}}}\"")));
        }
        assert!(!settings.contains("(устаревшая)"));
    }
}
