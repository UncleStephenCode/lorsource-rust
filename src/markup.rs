use html_escape::encode_text;
use once_cell::sync::Lazy;
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;

// Quotes and angle brackets are excluded from the URL match so a malicious
// `"` in a posted URL (e.g. `http://x" onmouseover="...`) can't be captured
// into the href attribute below and break out of it.
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("url regex"));
static USER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@([A-Za-z0-9_][A-Za-z0-9_.-]{1,79})" ).expect("user regex"));

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

pub fn render_message_with_markup(source: &str, markup: Option<&str>, bbcode: Option<bool>) -> String {
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
        ("[b]", "<strong>"), ("[/b]", "</strong>"),
        ("[i]", "<em>"), ("[/i]", "</em>"),
        ("[s]", "<s>"), ("[/s]", "</s>"),
        ("[u]", "<u>"), ("[/u]", "</u>"),
        ("[quote]", "<blockquote>"), ("[/quote]", "</blockquote>"),
        ("[code]", "<pre><code>"), ("[/code]", "</code></pre>"),
    ];
    for (from, to) in replacements {
        text = text.replace(from, to);
    }
    text = URL_RE.replace_all(&text, |caps: &regex::Captures| {
        let url = &caps[0];
        format!("<a href=\"{url}\" rel=\"nofollow ugc\">{url}</a>")
    }).to_string();
    text = USER_RE.replace_all(&text, |caps: &regex::Captures| {
        let nick = &caps[1];
        format!("<a href=\"/people/{nick}\">@{nick}</a>")
    }).to_string();
    text.split("\n\n")
        .map(|p| {
            let p = if user_line_break { p.replace('\n', "<br>") } else { p.to_string() };
            if p.starts_with("<blockquote>") || p.starts_with("<pre>") { p } else { format!("<p>{p}</p>") }
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
        assert!(render_message_with_markup("**жирный**", Some("MARKDOWN"), None).contains("<strong>жирный</strong>"));
        assert!(!render_message_with_markup("строка 1\nстрока 2", Some("BBCODE_TEX"), None).contains("<br>"));
        assert!(render_message_with_markup("строка 1\nстрока 2", Some("BBCODE_ULB"), None).contains("<br>"));
        assert!(render_message_with_markup("<b>текст</b>", Some("PLAIN"), None).contains("<b>текст</b>"));
    }
}
