use std::ops::Range;

use once_cell::sync::Lazy;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd, html};
use regex::Regex;

use crate::domain::markup::model::StMarkupUserDirectory;

mod lorcode;

// Quotes and angle brackets are excluded from the URL match so a malicious
// `"` in a posted URL (e.g. `http://x" onmouseover="...`) can't be captured
// into the href attribute below and break out of it.
// LorUserParserExtension.LorUser.  Notification extraction must use
// Flexmark's exact first-character and length rules rather than the old
// compact renderer's permissive mention expression.
static MARKDOWN_MENTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"@([A-Za-z][A-Za-z0-9_-]{0,80})").expect("markdown mention regex"));
// Parser.BBTAG_REGEXP, restricted only by the same lexical grammar (known-tag
// handling is not needed to find a normal MemberTag, but code/inline and
// escaped tags do need to be distinguished).
static LOR_TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\[\[?/?([a-z*]+)(?::[a-f0-9]+)?(?:=[^\]]+)?\]?\]").expect("LOR tag regex")
});
#[derive(Debug, Clone)]
enum EnCutPiece {
    Text(String),
    Cut {
        content: String,
        label: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StMarkdownMention {
    sNick: String,
    oSourceRange: Range<usize>,
}

/// MessageTextService.mentions: extract only the references which the parser
/// for the stored markup mode turns into mention AST nodes.  The returned
/// names are deduplicated in first-seen order; user existence and ignore-list
/// checks remain the responsibility of the write transaction.
pub fn extract_mentions(source: &str, markup: &str) -> Vec<String> {
    match markup {
        "MARKDOWN" => extract_markdown_mentions(source),
        "BBCODE_TEX" | "BBCODE_ULB" | "LORCODE" => extract_lorcode_mentions(source),
        // MarkupType.Html (stored as PLAIN) never casts users.  Unknown modes
        // are also fail-closed rather than reviving the old raw-text regex.
        _ => Vec::new(),
    }
}

/// MemberTag does not add a blocked user to RootNode.replier, while the
/// Flexmark visitor resolves a LorUser node regardless of the user's blocked
/// flag.  Callers use this when resolving extracted names in PostgreSQL.
pub fn mentions_include_blocked_users(markup: &str) -> bool {
    markup == "MARKDOWN"
}

fn extract_markdown_mentions(source: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for stMention in vecMarkdownMentions(source) {
        push_java_login_once(&mut out, &mut seen, &stMention.sNick);
    }
    out
}

fn vecMarkdownMentions(source: &str) -> Vec<StMarkdownMention> {
    let mut vecMentions = Vec::new();

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let mut code_block_depth = 0usize;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Text(_) if code_block_depth == 0 => {
                // Scan the original source slice rather than pulldown-cmark's
                // decoded CowStr.  That preserves Flexmark's boundary rule,
                // and prevents escaped '@' or an HTML entity from becoming a
                // notification after Markdown decoding.
                let raw = &source[range.clone()];
                for captures in MARKDOWN_MENTION_RE.captures_iter(raw) {
                    let whole = captures.get(0).expect("whole mention capture");
                    let absolute_start = range.start + whole.start();
                    if !is_markdown_mention_boundary(source, absolute_start) {
                        continue;
                    }
                    let nick = captures.get(1).expect("nick mention capture").as_str();
                    vecMentions.push(StMarkdownMention {
                        sNick: nick.to_owned(),
                        oSourceRange: absolute_start..range.start + whole.end(),
                    });
                }
            }
            // Event::Code covers inline code. Html/InlineHtml and link
            // destinations are not Text events, so they cannot cast users.
            _ => {}
        }
    }
    vecMentions
}

fn is_markdown_mention_boundary(source: &str, at: usize) -> bool {
    at == 0
        || source[..at]
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, ' ' | '(' | '\t' | '\n' | '\r'))
}

fn extract_lorcode_mentions(source: &str) -> Vec<String> {
    // This mirrors the observable MemberTag path for normal and unclosed
    // `[user]text[/user]` nodes.  Recovery from a malformed foreign closing
    // tag nested inside MemberTag is deliberately fail-closed: the compact
    // port may omit that cast, but must never invent one from code or text.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut code_mode = false;
    let mut active_user_text_start = None::<usize>;

    for captures in LOR_TAG_RE.captures_iter(source) {
        let whole = captures.get(0).expect("whole LOR tag capture");
        let raw_tag = whole.as_str();
        let tag = captures.get(1).expect("LOR tag name capture").as_str();
        let escaped = raw_tag.starts_with("[[") && raw_tag.ends_with("]]");
        let closing = raw_tag.starts_with("[/") || raw_tag.starts_with("[[/");

        if escaped {
            // processEscapedTag creates a TextNode.  If it is inside [user],
            // it becomes (or splits) the first child and therefore cannot be
            // a valid login unless the preceding raw text already was one.
            take_lor_user_text(
                source,
                &mut active_user_text_start,
                whole.start(),
                &mut out,
                &mut seen,
            );
            continue;
        }

        let is_code_tag = tag.eq_ignore_ascii_case("code") || tag.eq_ignore_ascii_case("inline");
        if code_mode {
            if is_code_tag && closing {
                code_mode = false;
            }
            continue;
        }
        if is_code_tag {
            take_lor_user_text(
                source,
                &mut active_user_text_start,
                whole.start(),
                &mut out,
                &mut seen,
            );
            if !closing {
                code_mode = true;
            }
            continue;
        }

        if tag.eq_ignore_ascii_case("user") {
            take_lor_user_text(
                source,
                &mut active_user_text_start,
                whole.start(),
                &mut out,
                &mut seen,
            );
            if !closing {
                active_user_text_start = Some(whole.end());
            }
        } else if active_user_text_start.is_some() {
            // MemberTag allows only a TextNode.  A tag token splits that
            // first child (or moves the parser out of MemberTag), so only the
            // text before it can be the name used by getUserCached.
            take_lor_user_text(
                source,
                &mut active_user_text_start,
                whole.start(),
                &mut out,
                &mut seen,
            );
        }
    }
    take_lor_user_text(
        source,
        &mut active_user_text_start,
        source.len(),
        &mut out,
        &mut seen,
    );
    out
}

fn take_lor_user_text(
    source: &str,
    start: &mut Option<usize>,
    end: usize,
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    if let Some(start) = start.take() {
        push_java_login_once(out, seen, source[start..end].trim());
    }
}

fn push_java_login_once(
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    nick: &str,
) {
    if is_java_login_name(nick) && seen.insert(nick.to_owned()) {
        out.push(nick.to_owned());
    }
}

fn is_java_login_name(nick: &str) -> bool {
    let bytes = nick.as_bytes();
    !bytes.is_empty()
        && bytes.len() < 80
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub fn render_message(source: &str, bbcode: Option<bool>) -> String {
    render_message_with_markup(source, None, bbcode)
}

pub fn render_message_with_markup(
    source: &str,
    markup: Option<&str>,
    bbcode: Option<bool>,
) -> String {
    render_message_with_markup_policy(source, markup, bbcode, false, None)
}

/// Render authored content with the Java `MessageTextService` nofollow flag.
/// `optSiteOrigin` is used only by LORCODE's URL formatter: Java never adds
/// nofollow to a link whose authority matches `SiteConfig.mainURI`.
pub fn render_message_with_markup_policy(
    source: &str,
    markup: Option<&str>,
    bbcode: Option<bool>,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
) -> String {
    render_message_with_markup_policy_and_users(
        source,
        markup,
        bbcode,
        bNofollow,
        optSiteOrigin,
        None,
    )
}

pub fn render_message_with_markup_policy_and_users(
    source: &str,
    markup: Option<&str>,
    bbcode: Option<bool>,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    let (html, bTrustedOutput) = match markup {
        Some("MARKDOWN") => (
            render_markdown_with_nofollow_and_users(source, bNofollow, optSiteOrigin, optUsers),
            true,
        ),
        Some("PLAIN") => (source.to_string(), false),
        Some("BBCODE_ULB") => (
            render_lor_markup_mode(source, true, bNofollow, optSiteOrigin, optUsers),
            true,
        ),
        Some("BBCODE_TEX" | "LORCODE") => (
            render_lor_markup_mode(source, false, bNofollow, optSiteOrigin, optUsers),
            true,
        ),
        _ if bbcode == Some(false) => (
            render_markdown_with_nofollow_and_users(source, bNofollow, optSiteOrigin, optUsers),
            true,
        ),
        _ => (
            render_lor_markup_mode(source, false, bNofollow, optSiteOrigin, optUsers),
            true,
        ),
    };
    if bTrustedOutput {
        // Both AST renderers emit only fixed tags and escaped
        // attributes/text. Keeping their post-sanitizer output intact is
        // required for the source-compatible user-reference inline styles
        // (and for LORCODE topic cut fragment ids).
        return html;
    }
    // Keep a final allow-list sanitizer as defence in depth for every mode.
    // Markdown additionally suppresses source HTML while parsing; legacy
    // PLAIN is intentionally treated as sanitized HTML rather than Markdown.
    sanitize_html(&html)
}

/// Topic preview in section feeds.  Java renders topic cuts collapsed in
/// `PreparedTopic` while the canonical topic page expands the same content.
pub fn render_topic_with_minimized_cut_policy_and_users(
    source: &str,
    markup: &str,
    canonical_url: &str,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    render_topic_cut(
        source,
        markup,
        canonical_url,
        true,
        bNofollow,
        optSiteOrigin,
        optUsers,
    )
}

pub fn render_topic_with_expanded_cut_policy_and_users(
    source: &str,
    markup: &str,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    render_topic_cut(
        source,
        markup,
        "",
        false,
        bNofollow,
        optSiteOrigin,
        optUsers,
    )
}

fn render_topic_cut(
    source: &str,
    markup: &str,
    canonical_url: &str,
    minimized: bool,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    if matches!(markup, "BBCODE_TEX" | "BBCODE_ULB" | "LORCODE") {
        let enCutMode = if minimized {
            lorcode::EnCutMode::TopicMinimized(canonical_url)
        } else {
            lorcode::EnCutMode::TopicExpanded
        };
        // The LORCODE renderer emits only fixed, escaped HTML elements.  Do
        // not run its result through the generic sanitizer here: Ammonia
        // removes the source-compatible `cutN` id used by fragment links.
        return lorcode::render(
            source,
            markup == "BBCODE_ULB",
            bNofollow,
            optSiteOrigin,
            optUsers,
            enCutMode,
        );
    }

    let (pieces, markdown) = if markup == "MARKDOWN" {
        (markdown_cut_pieces(source), true)
    } else {
        return render_message_with_markup_policy_and_users(
            source,
            Some(markup),
            None,
            bNofollow,
            optSiteOrigin,
            optUsers,
        );
    };
    // FlexmarkMarkdownFormatter.renderWithMinimizedCut ignores its nofollow
    // argument and constructs options with nofollow=false.
    let bEffectiveNofollow = bNofollow && !(minimized && markdown);
    if !pieces
        .iter()
        .any(|piece| matches!(piece, EnCutPiece::Cut { .. }))
    {
        return render_message_with_markup_policy_and_users(
            source,
            Some(markup),
            None,
            bEffectiveNofollow,
            optSiteOrigin,
            optUsers,
        );
    }

    let mut html = String::new();
    let mut cut_index = 0usize;
    for piece in pieces {
        match piece {
            EnCutPiece::Text(text) => {
                html.push_str(&render_message_with_markup_policy_and_users(
                    &text,
                    Some(markup),
                    None,
                    bEffectiveNofollow,
                    optSiteOrigin,
                    optUsers,
                ));
            }
            EnCutPiece::Cut { content, label } => {
                let anchor = if markdown {
                    if cut_index == 0 {
                        "cut".to_owned()
                    } else {
                        format!("cut-{cut_index}")
                    }
                } else {
                    format!("cut{cut_index}")
                };
                if minimized {
                    let href_value = format!("{canonical_url}#{anchor}");
                    let href = html_escape::encode_double_quoted_attribute(&href_value);
                    let label = label
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| html_escape::encode_text(value.trim()).into_owned())
                        .unwrap_or_else(|| "читать дальше...".to_owned());
                    html.push_str(&format!("<p>( <a href=\"{href}\">{label}</a> )</p>"));
                } else {
                    html.push_str(&format!("<div id=\"{anchor}\">"));
                    html.push_str(&render_message_with_markup_policy_and_users(
                        &content,
                        Some(markup),
                        None,
                        bEffectiveNofollow,
                        optSiteOrigin,
                        optUsers,
                    ));
                    html.push_str("</div>");
                }
                cut_index += 1;
            }
        }
    }
    html
}

fn markdown_cut_pieces(source: &str) -> Vec<EnCutPiece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut cut = None::<String>;
    for line in source.split_inclusive('\n') {
        let marker = line.trim_end_matches(['\r', '\n']);
        if cut.is_none() && marker == ">>>" {
            if !text.is_empty() {
                pieces.push(EnCutPiece::Text(std::mem::take(&mut text)));
            }
            cut = Some(String::new());
        } else if marker == "<<<" {
            if let Some(content) = cut.take() {
                pieces.push(EnCutPiece::Cut {
                    content,
                    label: None,
                });
            } else {
                text.push_str(line);
            }
        } else if let Some(content) = cut.as_mut() {
            content.push_str(line);
        } else {
            text.push_str(line);
        }
    }
    if let Some(content) = cut {
        pieces.push(EnCutPiece::Cut {
            content,
            label: None,
        });
    }
    if !text.is_empty() {
        pieces.push(EnCutPiece::Text(text));
    }
    pieces
}

fn sanitize_html(html: &str) -> String {
    ammonia::Builder::default()
        .add_tag_attributes("a", &["rel"])
        .link_rel(None)
        .clean(html)
        .to_string()
}

pub fn render_markdown(source: &str) -> String {
    render_markdown_with_nofollow(source, false)
}

pub fn render_markdown_with_nofollow(source: &str, bNofollow: bool) -> String {
    render_markdown_with_nofollow_and_users(source, bNofollow, None, None)
}

fn render_markdown_with_nofollow_and_users(
    source: &str,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    let (sPrepared, optMarkerPrefix, vecMentions) =
        stPrepareMarkdownUserMarkers(source, optUsers.is_some());
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let mut image_link_stack = Vec::new();
    let mut nofollow_link_stack = Vec::new();
    let parser = Parser::new_ext(&sPrepared, options).filter_map(move |event| match event {
        // Flexmark uses HtmlRenderer.SUPPRESS_HTML: HTML authored in Markdown
        // is discarded rather than passed through to the output sanitizer.
        Event::Html(_) | Event::InlineHtml(_) => None,
        // LOR's SuppressImagesExtension renders inline Markdown images as
        // nofollow links.  This prevents an external image URL from becoming
        // an automatically requested tracking pixel.  The image title is
        // deliberately not copied, matching the original renderer.
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            ..
        }) => {
            let is_inline = link_type == LinkType::Inline;
            image_link_stack.push(is_inline);
            if !is_inline {
                return None;
            }
            let href = html_escape::encode_double_quoted_attribute(&dest_url);
            Some(Event::Html(
                format!("<a href=\"{href}\" rel=\"nofollow\">").into(),
            ))
        }
        Event::End(TagEnd::Image) => image_link_stack
            .pop()
            .unwrap_or(false)
            .then(|| Event::Html("</a>".into())),
        Event::Start(Tag::Link {
            ref dest_url,
            ref title,
            ..
        }) => {
            nofollow_link_stack.push(bNofollow);
            if !bNofollow {
                return Some(event);
            }
            let href = html_escape::encode_double_quoted_attribute(&dest_url);
            let sTitle = if title.is_empty() {
                String::new()
            } else {
                format!(
                    " title=\"{}\"",
                    html_escape::encode_double_quoted_attribute(&title)
                )
            };
            Some(Event::Html(
                format!("<a href=\"{href}\"{sTitle} rel=\"nofollow\">").into(),
            ))
        }
        Event::End(TagEnd::Link) => {
            if nofollow_link_stack.pop().unwrap_or(false) {
                Some(Event::Html("</a>".into()))
            } else {
                Some(Event::End(TagEnd::Link))
            }
        }
        _ => Some(event),
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    // The generated image link still goes through the protocol allow-list,
    // so a javascript: image target cannot turn into an executable href.
    let sSanitized = sanitize_html(&out);
    sRestoreMarkdownUserMarkers(
        &sSanitized,
        optMarkerPrefix.as_deref(),
        &vecMentions,
        optSiteOrigin,
        optUsers,
    )
}

fn stPrepareMarkdownUserMarkers(
    source: &str,
    bResolveUsers: bool,
) -> (String, Option<String>, Vec<StMarkdownMention>) {
    if !bResolveUsers {
        return (source.to_owned(), None, Vec::new());
    }
    let vecMentions = vecMarkdownMentions(source);
    if vecMentions.is_empty() {
        return (source.to_owned(), None, vecMentions);
    }

    // Use an authored-text-safe private-use prefix which cannot collide with
    // the message. Each marker carries its source-order index, so a marker
    // suppressed by Markdown context cannot shift later user resolutions.
    let mut sMarkerPrefix = "\u{e000}LORUSER".to_owned();
    while source.contains(&sMarkerPrefix) {
        sMarkerPrefix.push('\u{e000}');
    }
    let mut sPrepared = String::with_capacity(source.len());
    let mut iCursor = 0usize;
    for (iMention, stMention) in vecMentions.iter().enumerate() {
        sPrepared.push_str(&source[iCursor..stMention.oSourceRange.start]);
        sPrepared.push_str(&sMarkerPrefix);
        sPrepared.push_str(&iMention.to_string());
        sPrepared.push('\u{e001}');
        iCursor = stMention.oSourceRange.end;
    }
    sPrepared.push_str(&source[iCursor..]);
    (sPrepared, Some(sMarkerPrefix), vecMentions)
}

fn sRestoreMarkdownUserMarkers(
    sHtml: &str,
    optMarkerPrefix: Option<&str>,
    vecMentions: &[StMarkdownMention],
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    let (Some(sMarkerPrefix), Some(stUsers)) = (optMarkerPrefix, optUsers) else {
        return sHtml.to_owned();
    };
    let mut sOut = String::with_capacity(sHtml.len());
    let mut sRest = sHtml;
    while let Some(iStart) = sRest.find(sMarkerPrefix) {
        sOut.push_str(&sRest[..iStart]);
        let sAfterPrefix = &sRest[iStart + sMarkerPrefix.len()..];
        let Some(iEnd) = sAfterPrefix.find('\u{e001}') else {
            sOut.push_str(&sRest[iStart..]);
            return sOut;
        };
        let optMention = sAfterPrefix[..iEnd]
            .parse::<usize>()
            .ok()
            .and_then(|iMention| vecMentions.get(iMention));
        if let Some(stMention) = optMention {
            sOut.push_str(&sRenderMarkdownUser(stMention, optSiteOrigin, stUsers));
        } else {
            sOut.push_str(sMarkerPrefix);
            sOut.push_str(&sAfterPrefix[..iEnd]);
            sOut.push('\u{e001}');
        }
        sRest = &sAfterPrefix[iEnd + '\u{e001}'.len_utf8()..];
    }
    sOut.push_str(sRest);
    sOut
}

fn sRenderMarkdownUser(
    stMention: &StMarkdownMention,
    optSiteOrigin: Option<&str>,
    stUsers: &StMarkupUserDirectory,
) -> String {
    let sEscapedNick = html_escape::encode_text(&stMention.sNick);
    let Some(stUser) = stUsers.optFind(&stMention.sNick) else {
        return format!("<s>@{sEscapedNick}</s>");
    };
    let sOrigin = optSiteOrigin.unwrap_or_default().trim_end_matches('/');
    let sHref = format!("{sOrigin}/people/{}/profile", stUser.sCanonicalNick);
    let sAnchor = format!(
        "<a href=\"{}\" class=\"mention\">@{sEscapedNick}</a>",
        html_escape::encode_double_quoted_attribute(&sHref)
    );
    if stUser.bBlocked {
        format!("<span style=\"white-space: nowrap\"><s>{sAnchor}</s></span>")
    } else {
        format!("<span style=\"white-space: nowrap\">{sAnchor}</span>")
    }
}

fn render_lor_markup_mode(
    source: &str,
    user_line_break: bool,
    bNofollow: bool,
    optSiteOrigin: Option<&str>,
    optUsers: Option<&StMarkupUserDirectory>,
) -> String {
    lorcode::render(
        source,
        user_line_break,
        bNofollow,
        optSiteOrigin,
        optUsers,
        lorcode::EnCutMode::Comment,
    )
}

fn bSameSiteUrl(sUrl: &str, optSiteOrigin: Option<&str>) -> bool {
    let Some(sSiteOrigin) = optSiteOrigin else {
        return false;
    };
    let (Ok(stUrl), Ok(stOrigin)) = (reqwest::Url::parse(sUrl), reqwest::Url::parse(sSiteOrigin))
    else {
        return false;
    };
    matches!(stUrl.scheme(), "http" | "https")
        && stUrl.host_str() == stOrigin.host_str()
        // `LorURL` compares Apache URI's raw ports.  `url::Url::port()`
        // normalizes an explicitly written default (`:80`/`:443`) to None,
        // which would incorrectly turn such a link into a local LOR URL.
        && optExplicitUrlPort(sUrl) == optExplicitUrlPort(sSiteOrigin)
}

fn optExplicitUrlPort(sUrl: &str) -> Option<u16> {
    let (_, sAfterScheme) = sUrl.split_once("://")?;
    let sAuthorityWithUser = sAfterScheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let sAuthority = sAuthorityWithUser
        .rsplit_once('@')
        .map_or(sAuthorityWithUser, |(_, sHost)| sHost);
    let sPort = if let Some(sAfterIpv6) = sAuthority.strip_prefix('[') {
        sAfterIpv6.split_once(']')?.1.strip_prefix(':')?
    } else {
        sAuthority.rsplit_once(':')?.1
    };
    if sPort.is_empty() || !sPort.bytes().all(|bByte| bByte.is_ascii_digit()) {
        return None;
    }
    sPort.parse().ok()
}

static TAG_STRIP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").expect("tag strip regex"));

/// Plain text suitable for a search index: render markup to HTML (through
/// the same sanitizing pipeline as normal display) then strip tags, so the
/// index holds readable text rather than raw BBCode/markdown source or
/// unrendered HTML.
pub fn plain_text_for_index(source: &str) -> String {
    let html = render_message(source, Some(true));
    let stripped = TAG_STRIP_RE.replace_all(&html, " ");
    stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_all_java_markup_modes() {
        assert!(
            render_message_with_markup("**жирный**", Some("MARKDOWN"), None)
                .contains("<strong>жирный</strong>")
        );
        assert!(
            !render_message_with_markup("строка 1\nстрока 2", Some("BBCODE_TEX"), None)
                .contains("<br>")
        );
        assert!(
            render_message_with_markup("строка 1\nстрока 2", Some("BBCODE_ULB"), None)
                .contains("<br>")
        );
        assert!(
            render_message_with_markup("<b>текст</b>", Some("PLAIN"), None)
                .contains("<b>текст</b>")
        );
    }

    #[test]
    fn topic_cuts_are_collapsed_in_feeds_and_expanded_on_topic_page() {
        let markdown = "до\n\n>>>\nскрыто\n<<<\nпосле";
        let collapsed = render_topic_with_minimized_cut_policy_and_users(
            markdown,
            "MARKDOWN",
            "/news/g/1",
            false,
            None,
            None,
        );
        assert!(
            collapsed.contains("href=\"/news/g/1#cut\"") && collapsed.contains("читать дальше")
        );
        assert!(!collapsed.contains("скрыто"));
        let expanded = render_topic_with_expanded_cut_policy_and_users(
            markdown, "MARKDOWN", false, None, None,
        );
        assert!(expanded.contains("<div id=\"cut\">") && expanded.contains("скрыто"));

        let lor = "до [cut=ещё]скрыто[/cut] после";
        let collapsed = render_topic_with_minimized_cut_policy_and_users(
            lor,
            "BBCODE_TEX",
            "/forum/g/2",
            false,
            None,
            None,
        );
        assert!(collapsed.contains("href=\"/forum/g/2#cut0\"") && collapsed.contains("ещё"));
        assert!(!collapsed.contains("скрыто"));
        let expanded =
            render_topic_with_expanded_cut_policy_and_users(lor, "BBCODE_TEX", false, None, None);
        assert!(expanded.contains("<div id=\"cut0\">") && expanded.contains("скрыто"));
    }

    #[test]
    fn markdown_suppresses_raw_html_elements() {
        let source = concat!(
            "before <img src=\"https://tracker.example/pixel.png\"> after\n\n",
            "<script>alert('script')</script>\n\n",
            "<form action=\"https://evil.example/collect\"><input name=\"secret\"></form>\n\n",
            "<svg onload=\"alert('svg')\"><circle /></svg>"
        );

        let rendered = render_message_with_markup(source, Some("MARKDOWN"), None);

        assert!(!rendered.contains("<img"), "{rendered}");
        assert!(!rendered.contains("<script"), "{rendered}");
        assert!(!rendered.contains("<form"), "{rendered}");
        assert!(!rendered.contains("<svg"), "{rendered}");
        assert!(!rendered.contains("tracker.example"), "{rendered}");
        assert!(!rendered.contains("evil.example"), "{rendered}");
    }

    #[test]
    fn markdown_external_image_is_a_nofollow_link_not_an_image() {
        let rendered = render_message_with_markup(
            "![tracking pixel](https://tracker.example/pixel.png)",
            Some("MARKDOWN"),
            None,
        );

        assert_eq!(
            rendered,
            "<p><a href=\"https://tracker.example/pixel.png\" rel=\"nofollow\">tracking pixel</a></p>\n"
        );
        assert!(!rendered.contains("<img"));
    }

    #[test]
    fn markdown_image_escapes_alt_and_url_and_discards_title() {
        let rendered = render_message_with_markup(
            "![a & \"quoted\"](https://tracker.example/pixel?a=1&amp;b=2 '\" onmouseover=\"alert(1)')",
            Some("MARKDOWN"),
            None,
        );

        assert!(
            rendered
                .contains("href=\"https://tracker.example/pixel?a=1&amp;b=2\" rel=\"nofollow\""),
            "{rendered}"
        );
        assert!(rendered.contains(">a &amp; \"quoted\"</a>"), "{rendered}");
        assert!(!rendered.contains("<img"), "{rendered}");
        assert!(!rendered.contains("title="), "{rendered}");
        assert!(!rendered.contains("onmouseover"), "{rendered}");

        let unsafe_protocol =
            render_message_with_markup("![unsafe](<javascript:alert(1)>)", Some("MARKDOWN"), None);
        assert!(unsafe_protocol.contains("unsafe"), "{unsafe_protocol}");
        assert!(
            !unsafe_protocol.contains("javascript:"),
            "{unsafe_protocol}"
        );
        assert!(!unsafe_protocol.contains("<img"), "{unsafe_protocol}");
    }

    #[test]
    fn markdown_normal_links_keep_normal_link_rendering() {
        let rendered = render_message_with_markup(
            "[safe](https://example.test/path?a=1&b=2 \"normal title\")",
            Some("MARKDOWN"),
            None,
        );

        assert_eq!(
            rendered,
            "<p><a href=\"https://example.test/path?a=1&amp;b=2\" title=\"normal title\">safe</a></p>\n"
        );
    }

    #[test]
    fn markdown_nofollow_matches_message_text_service_flag() {
        let source = concat!(
            "[safe](https://example.test/path \"normal title\") and ",
            "<https://auto.test/> and ",
            "[same site](https://www.linux.org.ru/forum/)"
        );
        let followed = render_message_with_markup_policy(
            source,
            Some("MARKDOWN"),
            None,
            false,
            Some("https://www.linux.org.ru"),
        );
        assert!(!followed.contains("rel=\"nofollow\""), "{followed}");

        let restricted = render_message_with_markup_policy(
            source,
            Some("MARKDOWN"),
            None,
            true,
            Some("https://www.linux.org.ru"),
        );
        assert_eq!(restricted.matches("rel=\"nofollow\"").count(), 3);
        assert!(
            restricted.contains(
                "href=\"https://example.test/path\" title=\"normal title\" rel=\"nofollow\""
            ),
            "{restricted}"
        );
        assert!(
            restricted.contains("href=\"https://auto.test/\" rel=\"nofollow\""),
            "{restricted}"
        );
        assert!(
            restricted.contains("href=\"https://www.linux.org.ru/forum/\" rel=\"nofollow\""),
            "Markdown's NofollowExtension does not exempt same-site links: {restricted}"
        );
    }

    #[test]
    fn minimized_markdown_preserves_flexmark_nofollow_quirk() {
        let source = "[external](https://example.test/)\n\n>>>\nhidden\n<<<";
        let collapsed = render_topic_with_minimized_cut_policy_and_users(
            source,
            "MARKDOWN",
            "https://www.linux.org.ru/forum/g/1",
            true,
            Some("https://www.linux.org.ru"),
            None,
        );
        assert!(!collapsed.contains("rel=\"nofollow\""), "{collapsed}");

        let expanded = render_topic_with_expanded_cut_policy_and_users(
            source,
            "MARKDOWN",
            true,
            Some("https://www.linux.org.ru"),
            None,
        );
        assert!(expanded.contains("rel=\"nofollow\""), "{expanded}");
    }

    #[test]
    fn lorcode_nofollow_exempts_same_authority_like_to_html_formatter() {
        let source = "https://outside.example/x https://www.linux.org.ru/forum/g/1";
        let restricted = render_message_with_markup_policy(
            source,
            Some("BBCODE_TEX"),
            None,
            true,
            Some("https://www.linux.org.ru"),
        );
        assert!(
            restricted.contains("href=\"https://outside.example/x\" rel=\"nofollow\""),
            "{restricted}"
        );
        assert!(
            restricted.contains(
                "href=\"https://www.linux.org.ru/forum/g/1\">https://www.linux.org.ru/forum/g/1</a>"
            ),
            "{restricted}"
        );

        let followed = render_message_with_markup_policy(
            source,
            Some("BBCODE_TEX"),
            None,
            false,
            Some("https://www.linux.org.ru"),
        );
        assert!(!followed.contains("rel=\"nofollow\""), "{followed}");

        let explicit_default_port = render_message_with_markup_policy(
            "https://www.linux.org.ru:443/forum/g/1",
            Some("BBCODE_TEX"),
            None,
            true,
            Some("https://www.linux.org.ru"),
        );
        assert!(
            explicit_default_port.contains("rel=\"nofollow\""),
            "LorURL compares an explicitly supplied port with mainURI's raw port: {explicit_default_port}"
        );
    }

    #[test]
    fn lorcode_member_tags_keep_exact_java_dom_and_do_not_inherit_nofollow() {
        use crate::domain::markup::model::{StMarkupUser, StMarkupUserDirectory};

        let stUsers = StMarkupUserDirectory::stFromUsers(vec![
            StMarkupUser {
                sInputNick: "crane2000".to_owned(),
                sCanonicalNick: "crane2000".to_owned(),
                bBlocked: false,
            },
            StMarkupUser {
                sInputNick: "bird50".to_owned(),
                sCanonicalNick: "bird50".to_owned(),
                bBlocked: true,
            },
        ]);
        let sRendered = render_message_with_markup_policy_and_users(
            "[user]crane2000[/user] [user]bird50[/user] [user]missing[/user] https://outside.example/",
            Some("BBCODE_TEX"),
            None,
            true,
            Some("https://www.linux.org.ru"),
            Some(&stUsers),
        );

        assert!(sRendered.contains(concat!(
            "<span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\">",
            "<a style=\"text-decoration: none\" href=\"https://www.linux.org.ru/people/crane2000/profile\">crane2000</a></span>"
        )));
        assert!(sRendered.contains(concat!(
            "<span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\"><s>",
            "<a style=\"text-decoration: none\" href=\"https://www.linux.org.ru/people/bird50/profile\">bird50</a></s></span>"
        )));
        assert!(sRendered.contains(" <s>missing</s>"));
        assert_eq!(sRendered.matches("rel=\"nofollow\"").count(), 1);
        assert!(sRendered.contains(
            "href=\"https://outside.example/\" rel=\"nofollow\">https://outside.example/</a>"
        ));
    }

    #[test]
    fn markdown_lor_users_keep_exact_java_dom_canonical_href_and_nofollow_scope() {
        use crate::domain::markup::model::{StMarkupUser, StMarkupUserDirectory};

        let stUsers = StMarkupUserDirectory::stFromUsers(vec![
            StMarkupUser {
                sInputNick: "Maxcom".to_owned(),
                sCanonicalNick: "maxcom".to_owned(),
                bBlocked: false,
            },
            StMarkupUser {
                sInputNick: "isden".to_owned(),
                sCanonicalNick: "isden".to_owned(),
                bBlocked: true,
            },
        ]);
        let sRendered = render_message_with_markup_policy_and_users(
            "@Maxcom @isden @hizel [outside](https://outside.example/)",
            Some("MARKDOWN"),
            None,
            true,
            Some("https://www.linux.org.ru/"),
            Some(&stUsers),
        );

        assert_eq!(
            sRendered,
            concat!(
                "<p><span style=\"white-space: nowrap\"><a href=\"https://www.linux.org.ru/people/maxcom/profile\" class=\"mention\">@Maxcom</a></span> ",
                "<span style=\"white-space: nowrap\"><s><a href=\"https://www.linux.org.ru/people/isden/profile\" class=\"mention\">@isden</a></s></span> ",
                "<s>@hizel</s> <a href=\"https://outside.example/\" rel=\"nofollow\">outside</a></p>\n"
            )
        );
        assert_eq!(sRendered.matches("rel=\"nofollow\"").count(), 1);
    }

    #[test]
    fn markdown_lor_user_renderer_respects_parser_context_and_duplicate_order() {
        use crate::domain::markup::model::{StMarkupUser, StMarkupUserDirectory};

        let stUsers = StMarkupUserDirectory::stFromUsers(vec![StMarkupUser {
            sInputNick: "alice".to_owned(),
            sCanonicalNick: "alice".to_owned(),
            bBlocked: false,
        }]);
        let sRendered = render_message_with_markup_policy_and_users(
            r"\@alice &#64;alice `@alice` @alice and @alice",
            Some("MARKDOWN"),
            None,
            false,
            Some("https://www.linux.org.ru"),
            Some(&stUsers),
        );

        assert_eq!(sRendered.matches("class=\"mention\"").count(), 2);
        assert!(sRendered.contains("@alice @alice <code>@alice</code>"));
    }

    #[test]
    fn legacy_html_ignores_message_text_service_nofollow_flag() {
        let rendered = render_message_with_markup_policy(
            "<a href=\"https://example.test/\">legacy</a>",
            Some("PLAIN"),
            None,
            true,
            Some("https://www.linux.org.ru"),
        );
        assert_eq!(rendered, "<a href=\"https://example.test/\">legacy</a>");
    }

    #[test]
    fn markdown_suppression_does_not_change_legacy_plain_html() {
        let rendered = render_message_with_markup(
            "<p>legacy <img src=\"https://example.test/legacy.png\"></p>",
            Some("PLAIN"),
            None,
        );

        assert!(rendered.contains("<img src=\"https://example.test/legacy.png\">"));
    }

    #[test]
    fn html_mode_never_extracts_mentions() {
        assert!(extract_mentions("<p>@alice [user]bob[/user]</p>", "PLAIN").is_empty());
        assert!(extract_mentions("@alice", "unknown").is_empty());
    }

    #[test]
    fn lorcode_extracts_only_member_tags_and_skips_code() {
        let source = concat!(
            "@raw ",
            "[user]alice[/user] ",
            "[USER]\n JB \n[/USER] ",
            "[user]alice[/user] ",
            "[code][user]inside_code[/user][/code] ",
            "[inline][user]inside_inline[/user][/inline] ",
            "[[user]]escaped[[/user]]"
        );

        assert_eq!(extract_mentions(source, "BBCODE_TEX"), vec!["alice", "JB"]);
        assert_eq!(
            extract_mentions("[user]alice[/user]", "BBCODE_ULB"),
            vec!["alice"]
        );
    }

    #[test]
    fn markdown_mentions_follow_flexmark_boundary_and_name_pattern() {
        let source = concat!(
            "@alice @a (@under_score)\t@dash-name\n@AfterBreak\r\n",
            "mail@example.test x@embedded comma,@not_boundary [@link_label](https://example.test) ",
            "@dot.stop @alice"
        );

        assert_eq!(
            extract_mentions(source, "MARKDOWN"),
            vec![
                "alice",
                "a",
                "under_score",
                "dash-name",
                "AfterBreak",
                "dot"
            ]
        );
    }

    #[test]
    fn markdown_mentions_ignore_inline_and_fenced_code_and_destinations() {
        let source = concat!(
            "visible @alice\n\n",
            "`@inline`\n\n",
            "```text\n@fenced\n```\n\n",
            "    @indented\n\n",
            "[plain link](https://example.test/@destination)"
        );

        assert_eq!(extract_mentions(source, "MARKDOWN"), vec!["alice"]);
    }

    #[test]
    fn mention_names_keep_java_case_sensitive_lookup_semantics() {
        assert_eq!(
            extract_mentions("@Alice @alice @Alice", "MARKDOWN"),
            vec!["Alice", "alice"]
        );
        assert!(extract_mentions(&format!("@{}", "a".repeat(80)), "MARKDOWN").is_empty());
    }

    #[test]
    fn mention_resolution_policy_matches_java_parser_types() {
        assert!(mentions_include_blocked_users("MARKDOWN"));
        assert!(!mentions_include_blocked_users("BBCODE_TEX"));
        assert!(!mentions_include_blocked_users("BBCODE_ULB"));
        assert!(!mentions_include_blocked_users("PLAIN"));
    }
}
