use html_escape::encode_text;
use once_cell::sync::Lazy;
use pulldown_cmark::{Options, Parser, html};
use regex::Regex;

// Quotes and angle brackets are excluded from the URL match so a malicious
// `"` in a posted URL (e.g. `http://x" onmouseover="...`) can't be captured
// into the href attribute below and break out of it.
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("url regex"));
static USER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"@([A-Za-z0-9_][A-Za-z0-9_.-]{1,79})").expect("user regex"));
static LOR_CUT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)\[cut(?:=([^\]]*))?\](.*?)\[/cut\]").expect("LOR cut regex"));

#[derive(Debug, Clone)]
enum EnCutPiece {
    Text(String),
    Cut {
        content: String,
        label: Option<String>,
    },
}

/// CommentCreateService.notifyMentions: pulls @nick references out of raw
/// (unrendered) message source, deduplicated, in first-seen order - used to
/// generate REFERENCE ("REF") notifications for mentioned users.
pub fn extract_mentions(source: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for caps in USER_RE.captures_iter(source) {
        let nick = caps[1].to_string();
        if seen.insert(nick.to_lowercase()) {
            out.push(nick);
        }
    }
    out
}

pub fn render_message(source: &str, bbcode: Option<bool>) -> String {
    render_message_with_markup(source, None, bbcode)
}

pub fn render_message_with_markup(
    source: &str,
    markup: Option<&str>,
    bbcode: Option<bool>,
) -> String {
    let html = match markup {
        Some("MARKDOWN") => render_markdown(source),
        Some("PLAIN") => source.to_string(),
        Some("BBCODE_ULB") => render_lor_markup_mode(source, true),
        Some("BBCODE_TEX" | "LORCODE") => render_lor_markup_mode(source, false),
        _ if bbcode == Some(false) => render_markdown(source),
        _ => render_lor_markup_mode(source, false),
    };
    // Matches MessageTextService's universal Jsoup.clean(text, Safelist.relaxed())
    // pass in the Java original: every rendering path - lorcode/BBCode,
    // markdown, and (if ever wired up) raw HTML mode - goes through an
    // allow-list HTML sanitizer as a final safety net, not just the ones
    // that are "supposed to" already be safe. This is what actually stops
    // pulldown-cmark's raw-HTML passthrough and closes any escaping bug in
    // the hand-rolled autolinker above from becoming stored XSS.
    sanitize_html(&html)
}

/// Topic preview in section feeds.  Java renders topic cuts collapsed in
/// `PreparedTopic` while the canonical topic page expands the same content.
pub fn render_topic_with_minimized_cut(source: &str, markup: &str, canonical_url: &str) -> String {
    render_topic_cut(source, markup, canonical_url, true)
}

pub fn render_topic_with_expanded_cut(source: &str, markup: &str) -> String {
    render_topic_cut(source, markup, "", false)
}

fn render_topic_cut(source: &str, markup: &str, canonical_url: &str, minimized: bool) -> String {
    let (pieces, markdown) = if markup == "MARKDOWN" {
        (markdown_cut_pieces(source), true)
    } else if matches!(markup, "BBCODE_TEX" | "BBCODE_ULB" | "LORCODE") {
        (lor_cut_pieces(source), false)
    } else {
        return render_message_with_markup(source, Some(markup), None);
    };
    if !pieces
        .iter()
        .any(|piece| matches!(piece, EnCutPiece::Cut { .. }))
    {
        return render_message_with_markup(source, Some(markup), None);
    }

    let mut html = String::new();
    let mut cut_index = 0usize;
    for piece in pieces {
        match piece {
            EnCutPiece::Text(text) => {
                html.push_str(&render_message_with_markup(&text, Some(markup), None));
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
                    html.push_str(&render_message_with_markup(&content, Some(markup), None));
                    html.push_str("</div>");
                }
                cut_index += 1;
            }
        }
    }
    html
}

fn lor_cut_pieces(source: &str) -> Vec<EnCutPiece> {
    let mut pieces = Vec::new();
    let mut offset = 0usize;
    for captures in LOR_CUT_RE.captures_iter(source) {
        let whole = captures.get(0).expect("whole cut capture");
        if whole.start() > offset {
            pieces.push(EnCutPiece::Text(source[offset..whole.start()].to_owned()));
        }
        pieces.push(EnCutPiece::Cut {
            label: captures.get(1).map(|value| value.as_str().to_owned()),
            content: captures
                .get(2)
                .map_or_else(String::new, |value| value.as_str().to_owned()),
        });
        offset = whole.end();
    }
    if offset < source.len() {
        pieces.push(EnCutPiece::Text(source[offset..].to_owned()));
    }
    pieces
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
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(source, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn render_lor_markup_mode(source: &str, user_line_break: bool) -> String {
    // This is a compact Rust replacement for the old formatter pipeline.
    // It intentionally keeps the supported subset explicit and safe:
    // paragraphs, quotes, code blocks, common BBCode tags, autolinks and @mentions.
    let mut text = encode_text(source).to_string();
    let replacements = [
        ("[b]", "<strong>"),
        ("[/b]", "</strong>"),
        ("[i]", "<em>"),
        ("[/i]", "</em>"),
        ("[s]", "<s>"),
        ("[/s]", "</s>"),
        ("[u]", "<u>"),
        ("[/u]", "</u>"),
        ("[quote]", "<blockquote>"),
        ("[/quote]", "</blockquote>"),
        ("[code]", "<pre><code>"),
        ("[/code]", "</code></pre>"),
    ];
    for (from, to) in replacements {
        text = text.replace(from, to);
    }
    text = URL_RE
        .replace_all(&text, |caps: &regex::Captures| {
            let url = &caps[0];
            format!("<a href=\"{url}\" rel=\"nofollow ugc\">{url}</a>")
        })
        .to_string();
    text = USER_RE
        .replace_all(&text, |caps: &regex::Captures| {
            let nick = &caps[1];
            format!("<a href=\"/people/{nick}\">@{nick}</a>")
        })
        .to_string();
    text.split("\n\n")
        .map(|p| {
            let p = if user_line_break {
                p.replace('\n', "<br>")
            } else {
                p.to_string()
            };
            if p.starts_with("<blockquote>") || p.starts_with("<pre>") {
                p
            } else {
                format!("<p>{p}</p>")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        let collapsed = render_topic_with_minimized_cut(markdown, "MARKDOWN", "/news/g/1");
        assert!(
            collapsed.contains("href=\"/news/g/1#cut\"") && collapsed.contains("читать дальше")
        );
        assert!(!collapsed.contains("скрыто"));
        let expanded = render_topic_with_expanded_cut(markdown, "MARKDOWN");
        assert!(expanded.contains("<div id=\"cut\">") && expanded.contains("скрыто"));

        let lor = "до [cut=ещё]скрыто[/cut] после";
        let collapsed = render_topic_with_minimized_cut(lor, "BBCODE_TEX", "/forum/g/2");
        assert!(collapsed.contains("href=\"/forum/g/2#cut0\"") && collapsed.contains("ещё"));
        assert!(!collapsed.contains("скрыто"));
        let expanded = render_topic_with_expanded_cut(lor, "BBCODE_TEX");
        assert!(expanded.contains("<div id=\"cut0\">") && expanded.contains("скрыто"));
    }
}
