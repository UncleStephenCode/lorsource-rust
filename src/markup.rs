use html_escape::encode_text;
use once_cell::sync::Lazy;
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;

static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://[^\s<]+" ).expect("url regex"));
static USER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@([A-Za-z0-9_][A-Za-z0-9_.-]{1,79})" ).expect("user regex"));

pub fn render_message(source: &str, bbcode: Option<bool>) -> String {
    if bbcode.unwrap_or(true) {
        render_lor_markup(source)
    } else {
        render_markdown(source)
    }
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
