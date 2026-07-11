use html_escape::encode_text;
use once_cell::sync::Lazy;
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;

// Quotes and angle brackets are excluded from the URL match so a malicious
// `"` in a posted URL (e.g. `http://x" onmouseover="...`) can't be captured
// into the href attribute below and break out of it.
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("url regex"));
static USER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@([A-Za-z0-9_][A-Za-z0-9_.-]{1,79})" ).expect("user regex"));

pub fn render_message(source: &str, bbcode: Option<bool>) -> String {
    let html = if bbcode.unwrap_or(true) {
        render_lor_markup(source)
    } else {
        render_markdown(source)
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

pub fn render_lor_markup(source: &str) -> String {
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
            let p = p.replace('\n', "<br>");
            if p.starts_with("<blockquote>") || p.starts_with("<pre>") { p } else { format!("<p>{p}</p>") }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn excerpt(source: &str, max_len: usize) -> String {
    let plain = Regex::new(r"\[[^\]]+\]").unwrap().replace_all(source, "");
    let mut out = plain.chars().take(max_len).collect::<String>();
    if plain.chars().count() > max_len {
        out.push('…');
    }
    out
}
