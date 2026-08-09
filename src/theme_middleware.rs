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

async fn resolve_profile(state: &AppState, jar: &CookieJar) -> (String, Option<String>) {
    if let Ok(Some(user_id)) = crate::auth::optUserIdFromCookies(
        &state.pool,
        jar,
        &state.config.site_secret,
        &state.config.cookie_secret,
    )
    .await
    {
        let profile: Option<(Option<String>, String)> = sqlx::query_as(
            "SELECT us.settings->'style', u.nick FROM users u LEFT JOIN user_settings us ON us.id=u.id WHERE u.id=$1",
        )
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        if let Some((style, nick)) = profile {
            return (
                style
                    .filter(|value| is_style(value))
                    .unwrap_or_else(|| DEFAULT_STYLE.to_string()),
                Some(nick),
            );
        }
    }
    (DEFAULT_STYLE.to_string(), None)
}

fn login_block(nick: Option<&str>, pony: bool) -> String {
    match nick {
        Some(nick) => {
            let greeting = if pony { "дружбомагии тебе" } else { "добро пожаловать" };
            format!(
                "{greeting}, <a style=\"text-decoration: none\" href=\"/people/{}/profile\">{}</a>",
                urlencoding::encode(nick),
                html_escape::encode_text(nick),
            )
        }
        None => "<div id=\"regmenu\" class=\"head\"><a href=\"/register.jsp\">Регистрация</a> - <a id=\"loginbutton\" href=\"/login.jsp\">Вход</a></div>".to_string(),
    }
}

fn modern_header(nick: Option<&str>) -> String {
    let top_profile = nick.map_or_else(String::new, |nick| {
        format!(
            "<a href=\"/notifications\"><i class=\"icon-bell\"></i><span id=\"main_events_count_number\"></span></a><a title=\"{}\" href=\"/people/{}/profile\"><i class=\"icon-user-circle-o\"></i></a>",
            html_escape::encode_double_quoted_attribute(nick),
            urlencoding::encode(nick),
        )
    });
    let guest = if nick.is_none() {
        login_block(None, false)
    } else {
        String::new()
    };
    format!(
        r#"<header id="hd"><div id="topProfile">{top_profile}</div><span id="sitetitle"><a href="/">LINUX.ORG.RU</a></span><nav class="menu"><div id="loginGreating">{guest}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/polls/">Опросы</a></li> <li><a href="/tracker/">Трекер</a></li> <li><a href="/search.jsp">Поиск</a></li></ul></nav></header><div style="clear: both"></div>"#,
    )
}

fn waltz_header(nick: Option<&str>) -> String {
    let top_profile = nick.map_or_else(String::new, |nick| {
        format!(
            r#"<a style="text-decoration: none" href="/people/{}/profile">{}</a>"#,
            urlencoding::encode(nick),
            html_escape::encode_text(nick)
        )
    });
    let guest = if nick.is_none() {
        login_block(None, false)
    } else {
        String::new()
    };
    let events = if nick.is_some() {
        r#"<li><a href="/notifications">Уведомления <span id="main_events_count"></span></a></li>"#
    } else {
        ""
    };
    format!(
        r#"<header id="hd"><div id="topProfile">{top_profile}</div><span id="sitetitle"><a href="/">LINUX.ORG.RU</a></span><nav class="menu"><div id="loginGreating">{guest}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></nav></header><div style="clear: both"></div>"#
    )
}

fn black_header(nick: Option<&str>, main_page: bool) -> String {
    let login = login_block(nick, false);
    if !main_page {
        let events = if nick.is_some() {
            r#"<a style="text-decoration:none" href="/notifications">Уведомления <span id="main_events_count"></span></a> - "#
        } else {
            ""
        };
        return format!(
            r#"<table border="0" cellspacing="0" cellpadding="0" width="100%" class="head"><tr><td rowspan="2" align="left"><a href="/"><img src="/black/lor-new.png" width="282" height="60" alt="Linux.org.ru"></a></td><td align="right">{login}</td></tr><tr><td align="right" valign="bottom"><a style="text-decoration:none" href="/news/">Новости</a> - <a style="text-decoration:none" href="/gallery/">Галерея</a> - <a style="text-decoration:none" href="/articles/">Статьи</a> - <a style="text-decoration:none" href="/forum/">Форум</a> - {events}<a style="text-decoration:none" href="/tracker/">Трекер</a> - <a style="text-decoration:none" href="/search.jsp">Поиск</a></td></tr></table>"#,
        );
    }
    let events = if nick.is_some() {
        r#"<a href="/notifications">Уведомления <span id="main_events_count"></span></a>"#
    } else {
        ""
    };
    format!(
        r#"<a href="/"><img style="float:left;border:0" src="/black/lorlogo-try.png" alt="Русская информация об ОС LINUX" width="270" height="208"></a><div id="hd"><div id="head-main"><table><tr><td><a href="/news/">Новости</a></td><td><a href="/tracker/">Трекер</a></td><td><a href="/about">О сервере</a></td></tr><tr><td><a href="/gallery/">Галерея</a></td><td><a href="/forum/">Форум</a></td><td>{events}</td></tr><tr><td><a href="/articles/">Статьи</a></td><td></td><td><a href="/search.jsp">Поиск</a></td></tr></table><br></div><div style="right:5px;text-align:right;top:5px;position:absolute" class="head">{login}</div></div>"#,
    )
}

fn white2_header(nick: Option<&str>) -> String {
    let login = nick.map_or_else(
        || login_block(None, false),
        |nick| format!(r#"добро пожаловать,&nbsp;<a style="text-decoration: none" href="/people/{}/profile">{}</a>"#, urlencoding::encode(nick), html_escape::encode_text(nick)),
    );
    let events = if nick.is_some() {
        r#"<li><a href="/notifications">Уведомления <span id="main_events_count"></span></a></li>"#
    } else {
        ""
    };
    format!(
        r#"<div id="hd"><div id="hdtux"><img src="/img/Tux.svg" height="100%" alt="Linux"></div><a id="sitetitle" href="/">LINUX.ORG.RU</a><ul class="menu"><li id="loginGreating">{login}</li> <li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></div><div style="clear: both"></div>"#,
    )
}

fn pony_header(nick: Option<&str>) -> String {
    let login = login_block(nick, true);
    let events = if nick.is_some() {
        r#"<li><a href="/notifications">Уведомления <span id="main_events_count"></span></a></li>"#
    } else {
        ""
    };
    format!(
        r#"<div id="hd"><a id="sitetitle" href="/"><img src="/zomg_ponies/img/twilight_logo.png" id="twilight_logo" alt="">PONY.ORG.RU</a><div class="menu"><div id="loginGreating">{login}</div><ul><li><a href="/news/">Новости</a></li> <li><a href="/gallery/">Галерея</a></li> <li><a href="/articles/">Статьи</a></li> <li><a href="/forum/">Форум</a></li> <li><a href="/tracker/">Трекер</a></li> {events} <li><a href="/search.jsp">Поиск</a></li></ul></div></div><div style="clear: both"></div>"#,
    )
}

fn render_header(style: &str, nick: Option<&str>, main_page: bool) -> String {
    match style {
        "black" => black_header(nick, main_page),
        "white2" => white2_header(nick),
        "waltz" => waltz_header(nick),
        "zomg_ponies" => pony_header(nick),
        _ => modern_header(nick),
    }
}

const THEME_INDICATOR: &str = r#"<div id="theme-indicator"><span class="theme-indicator-dark" title="Темная тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="M9.37,5.51C9.19,6.15,9.1,6.82,9.1,7.5c0,4.08,3.32,7.4,7.4,7.4c0.68,0,1.35-0.09,1.99-0.27C17.45,17.19,14.93,19,12,19 c-3.86,0-7-3.14-7-7C5,9.07,6.81,6.55,9.37,5.51z M12,3c-4.97,0-9,4.03-9,9s4.03,9,9,9s9-4.03,9-9c0-0.46-0.04-0.92-0.1-1.36 c-0.98,1.37-2.58,2.26-4.4,2.26c-2.98,0-5.4-2.42-5.4-5.4c0-1.81,0.89-3.42,2.26-4.4C12.92,3.04,12.46,3,12,3L12,3z"></path></svg></span><span class="theme-indicator-light" title="Светлая тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="M12,9c1.65,0,3,1.35,3,3s-1.35,3-3,3s-3-1.35-3-3S10.35,9,12,9 M12,7c-2.76,0-5,2.24-5,5s2.24,5,5,5s5-2.24,5-5 S14.76,7,12,7L12,7z M2,13l2,0c0.55,0,1-0.45,1-1s-0.45-1-1-1l-2,0c-0.55,0-1,0.45-1,1S1.45,13,2,13z M20,13l2,0c0.55,0,1-0.45,1-1 s-0.45-1-1-1l-2,0c-0.55,0-1,0.45-1,1S19.45,13,20,13z M11,2v2c0,0.55,0.45,1,1,1s1-0.45,1-1V2c0-0.55-0.45-1-1-1S11,1.45,11,2z M11,20v2c0,0.55,0.45,1,1,1s1-0.45,1-1v-2c0-0.55-0.45-1-1-1C11.45,19,11,19.45,11,20z M5.99,4.58c-0.39-0.39-1.03-0.39-1.41,0 c-0.39,0.39-0.39,1.03,0,1.41l1.06,1.06c0.39,0.39,1.03,0.39,1.41,0s0.39-1.03,0-1.41L5.99,4.58z M18.36,16.95 c-0.39-0.39-1.03-0.39-1.41,0c-0.39,0.39-0.39,1.03,0,1.41l1.06,1.06c0.39,0.39,1.03,0.39,1.41,0c0.39-0.39,0.39-1.03,0-1.41 L18.36,16.95z M19.42,5.99c0.39-0.39,0.39-1.03,0-1.41c-0.39,0.39-1.03-0.39-1.41,0l-1.06,1.06c-0.39,0.39-0.39,1.03,0,1.41 s1.03,0.39,1.41,0L19.42,5.99z M7.05,18.36c0.39-0.39,0.39-1.03,0-1.41c-0.39-0.39-1.03-0.39-1.41,0l-1.06,1.06 c-0.39,0.39-0.39,1.03,0,1.41s1.03,0.39,1.41,0L7.05,18.36z"></path></svg></span><span class="theme-indicator-auto" title="Системная тема"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><path fill="currentColor" d="m12 21c4.971 0 9-4.029 9-9s-4.029-9-9-9-9 4.029-9 9 4.029 9 9 9zm4.95-13.95c1.313 1.313 2.05 3.093 2.05 4.95s-0.738 3.637-2.05 4.95c-1.313 1.313-3.093 2.05-4.95 2.05v-14c1.857 0 3.637 0.737 4.95 2.05z"></path></svg></span></div>"#;

fn render_footer(public_url: &str, ws_url: &str, authenticated: bool, main_page: bool) -> String {
    let info = if main_page {
        r#"<p id="ft-info"><a href="/about">О Сервере</a> - <a href="/help/rules.md">Правила форума</a><br>Разработка и&nbsp;поддержка&nbsp;— <a href="/people/maxcom/profile">Максим Валянский</a> 1998–2026<br>Сервер для сайта предоставлен «<a href="http://www.ittelo.ru/" target="_blank">ITTelo</a>»<br>Размещение сервера и&nbsp;подключение к&nbsp;сети Интернет осуществляется компанией «<a href="https://selectel.ru/?ref_code=3dce4333ba" target="_blank">Selectel</a>».</p>"#.to_string()
    } else {
        let url = html_escape::encode_double_quoted_attribute(public_url);
        format!(
            r#"<p id="ft-info"><a href="/about">О Сервере</a> - <a href="/help/rules.md">Правила форума</a><br><a href="https://github.com/maxcom/lorsource/issues">Сообщить об ошибке</a><br><a href="{url}">{url}</a></p>"#
        )
    };
    let theme_indicator = THEME_INDICATOR.replace(
        "c-0.39,0.39-1.03-0.39-1.41,0",
        "c-0.39-0.39-1.03-0.39-1.41,0",
    );
    let realtime = if authenticated {
        let sWsUrl = serde_json::to_string(ws_url).expect("serializing a string cannot fail");
        format!(
            r#"<script>$script.ready('realtime', function() {{ RealtimeContext.start({sWsUrl}); }});</script>"#
        )
    } else {
        String::new()
    };
    format!(r#"<footer id="ft">{info}{realtime}{theme_indicator}</footer>"#)
}

pub async fn apply_theme(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let main_page = matches!(req.uri().path(), "/" | "/index.jsp");
    let (style, nick) = resolve_profile(&state, &jar).await;
    let response = next.run(req).await;
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    if !is_html {
        return response;
    }

    let view = theme_view(&style);
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
    text = text.replacen(
        "<!-- LOR_THEME_HEADER -->",
        &render_header(&style, nick.as_deref(), main_page),
        1,
    );
    text = text.replacen(
        "<!-- LOR_THEME_FOOTER -->",
        &render_footer(
            &state.config.public_url,
            &state.config.ws_url,
            nick.is_some(),
            main_page,
        ),
        1,
    );
    let stTimezone = request_timezone::stRequestTimezone(&jar);
    text = text.replacen("<!-- LOR_TIMEZONE -->", stTimezone.name(), 1);
    text = request_timezone::sRewriteHtmlTimes(&text, stTimezone, chrono::Utc::now());
    if style == "black" && main_page {
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
        assert_eq!(black_header(None, false).matches("lor-new.png").count(), 1);
        assert_eq!(
            black_header(None, true).matches("lorlogo-try.png").count(),
            1
        );
        assert!(!black_header(None, true).contains("Уведомления"));
        assert!(black_header(Some("user"), true).contains("Уведомления"));
        assert!(
            !render_footer("https://example/", "wss://example/", false, false)
                .contains("RealtimeContext.start")
        );
        let authenticated_footer =
            render_footer("https://example/", "wss://example/ws-root/", true, false);
        assert!(authenticated_footer.contains("$script.ready('realtime'"));
        assert!(authenticated_footer.contains("RealtimeContext.start(\"wss://example/ws-root/\")"));
    }

    #[test]
    fn inline_menu_items_keep_the_jsp_whitespace_gap() {
        for header in [
            modern_header(None),
            waltz_header(None),
            white2_header(None),
            pony_header(None),
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
        assert!(!settings.contains("localStorage.setItem('lor-theme'"));
        assert!(!settings.contains("lor_theme="));
    }
}
