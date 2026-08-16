//! Compatibility helpers for the legacy title storage and presentation
//! contracts.
//!
//! Java stores topic titles after `StringUtil.escapeHtml`, which delegates to
//! Guava's `HtmlEscapers.htmlEscaper()`.  The edit form reverses exactly one
//! HTML-entity layer before presenting the value to the user. Public HTML has
//! several distinct pipelines: ordinary topics pass through `makeTitle` and
//! `TitleTag`/`processTitle`, while a few raw DAO projections only pass through
//! `TitleTag` or are written directly by JSP EL.

/// Escape a raw topic title exactly like Guava's `HtmlEscapers.htmlEscaper()`.
pub fn sEscapeForStorage(sRawTitle: &str) -> String {
    let mut sStoredTitle = String::with_capacity(sRawTitle.len());

    for cCharacter in sRawTitle.chars() {
        match cCharacter {
            '&' => sStoredTitle.push_str("&amp;"),
            '"' => sStoredTitle.push_str("&quot;"),
            '\'' => sStoredTitle.push_str("&#39;"),
            '<' => sStoredTitle.push_str("&lt;"),
            '>' => sStoredTitle.push_str("&gt;"),
            _ => sStoredTitle.push(cCharacter),
        }
    }

    sStoredTitle
}

/// Decode one HTML entity layer for an edit form, as Java's
/// `StringEscapeUtils.unescapeHtml4` does for titles written by Guava.
pub fn sUnescapeFromStorage(sStoredTitle: &str) -> String {
    html_escape::decode_html_entities(sStoredTitle).into_owned()
}

/// Produce plain text for an HTML presentation surface. Callers must still
/// use Askama's normal escaping or an `html_escape::encode_*` function for the
/// concrete HTML context; the returned value is deliberately not trusted HTML.
pub fn sPlainForDisplay(sStoredTitle: &str) -> String {
    sUnescapeFromStorage(sStoredTitle)
}

/// Reproduce `StringUtil.processTitle` followed by the browser's one-layer
/// entity decoding. This is the contract used by raw DAO values rendered
/// through `<l:title>` (for example the top-10/articles boxlets).
pub fn sProcessTitlePlainForDisplay(sTitle: &str) -> String {
    sUnescapeFromStorage(&sProcessTitleHtml(sTitle))
}

/// Reproduce `StringUtil.makeTitle` followed by the browser's one-layer entity
/// decoding. The poll boxlet uses a `Topic` title directly without
/// `<l:title>`, so it intentionally does not run `processTitle`.
pub fn sMakeTitlePlainForDisplay(sStoredTitle: &str) -> String {
    sUnescapeFromStorage(&sMakeTitleForLegacyView(sStoredTitle))
}

/// Plain DOM text for a regular Java `Topic`: `Topic.fromResultSet` first runs
/// `StringUtil.makeTitle`, and the JSP then runs `TitleTag`/`processTitle`.
/// Askama must escape the returned plain text normally; it is never safe HTML.
pub fn sTopicTitlePlainForDisplay(sStoredTitle: &str) -> String {
    sUnescapeFromStorage(&sProcessTitleHtml(&sMakeTitleForLegacyView(sStoredTitle)))
}

/// Plain DOM text for a legacy comment title. `PreparedComment` trims and
/// unescapes one layer, while its JSP `<c:out>` + `<l:title>` combination is
/// observably equivalent to processing the raw stored value and letting the
/// browser decode one layer. Whitespace-only titles remain absent.
pub fn optCommentTitlePlainForDisplay(sStoredTitle: &str) -> Option<String> {
    let sTrimmedTitle = sJavaTrim(sStoredTitle);
    (!sTrimmedTitle.is_empty()).then(|| sProcessTitlePlainForDisplay(sTrimmedTitle))
}

/// Exact entity-bearing result of Java `StringUtil.processTitle`. This is for
/// legacy pipelines which perform their own later XML/HTML handling.
pub fn sProcessTitleForLegacyView(sTitle: &str) -> String {
    sJavaTrim(sTitle).replace(" -- ", "&nbsp;&mdash; ")
}

fn sProcessTitleHtml(sTitle: &str) -> String {
    sProcessTitleForLegacyView(sTitle)
}

/// Exact entity-bearing result of Java `StringUtil.makeTitle`. This is exposed
/// for legacy non-HTML presentation pipelines (notably RSS), which must apply
/// their own subsequent XML escaping rather than browser decoding.
pub fn sMakeTitleForLegacyView(sTitle: &str) -> String {
    if sJavaTrim(sTitle).is_empty() {
        return "Без заглавия".to_owned();
    }

    // Exact `new RuTypoChanger().format(title)` pipeline. A new changer is
    // constructed by StringUtil for every title, hence its local buffer is
    // empty while this single string is scanned.
    let mut vecChars: Vec<char> = sTitle.replace("&quot;", "\"").chars().collect();
    let mut iQuoteDepth = 0_u32;
    for iPosition in 0..vecChars.len() {
        if vecChars[iPosition] != '"' {
            continue;
        }

        if bClosingQuote(&vecChars, iPosition) && iQuoteDepth > 0 {
            vecChars[iPosition] = if iQuoteDepth == 1 { '»' } else { '“' };
            iQuoteDepth -= 1;
        } else if bOpeningQuote(&vecChars, iPosition) {
            vecChars[iPosition] = if iQuoteDepth == 0 { '«' } else { '„' };
            iQuoteDepth += 1;
        }
    }

    vecChars
        .into_iter()
        .collect::<String>()
        .replace("''", "&quot;")
        .replace('"', "&quot;")
        .replace('„', "&#8222;")
        .replace('“', "&#8220;")
        .replace('«', "&#171;")
        .replace('»', "&#187;")
}

fn sJavaTrim(sValue: &str) -> &str {
    // java.lang.String.trim() removes UTF-16 code units <= U+0020, unlike
    // Rust's Unicode-aware str::trim().
    sValue.trim_matches(|cCharacter| cCharacter <= '\u{20}')
}

fn bQuote(cCharacter: char) -> bool {
    matches!(cCharacter, '"' | '«' | '»' | '„' | '“')
}

fn bPunctuation(cCharacter: char) -> bool {
    matches!(
        cCharacter,
        '.' | ',' | ':' | ';' | '-' | '!' | '?' | '(' | ')'
    )
}

fn optPreviousNonQuote(vecChars: &[char], iPosition: usize) -> Option<char> {
    vecChars[..iPosition]
        .iter()
        .rev()
        .copied()
        .find(|cCharacter| !bQuote(*cCharacter))
}

fn optNextNonQuote(vecChars: &[char], iPosition: usize) -> Option<char> {
    vecChars[iPosition + 1..]
        .iter()
        .copied()
        .find(|cCharacter| !bQuote(*cCharacter))
}

fn bOpeningQuote(vecChars: &[char], iPosition: usize) -> bool {
    if iPosition + 1 == vecChars.len() {
        return false;
    }

    let cBefore = if iPosition == 0 {
        '\0'
    } else {
        optPreviousNonQuote(vecChars, iPosition).unwrap_or(vecChars[0])
    };
    let cAfter = optNextNonQuote(vecChars, iPosition).unwrap_or(vecChars[vecChars.len() - 1]);

    !cAfter.is_whitespace() && !bPunctuation(cAfter) && !cBefore.is_alphanumeric()
}

fn bClosingQuote(vecChars: &[char], iPosition: usize) -> bool {
    if iPosition == 0 {
        return false;
    }
    if iPosition + 1 == vecChars.len() {
        return true;
    }

    let cBefore = optPreviousNonQuote(vecChars, iPosition).unwrap_or(vecChars[0]);
    let cAfter = optNextNonQuote(vecChars, iPosition).unwrap_or(vecChars[vecChars.len() - 1]);
    !bQuote(cBefore) && !cAfter.is_alphanumeric()
}

/// Java's `String.length()` counts UTF-16 code units, not Unicode scalar
/// values. Title validation must use the same unit to preserve its 140-unit
/// boundary.
pub fn iJavaStringLength(sValue: &str) -> usize {
    sValue.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use askama::Template;

    use super::*;

    #[derive(Template)]
    #[template(
        source = "<h1>{{ sTitle }}</h1><meta property=\"og:title\" content=\"{{ sTitle }}\">",
        ext = "html"
    )]
    struct StPlainTitleTemplate<'a> {
        sTitle: &'a str,
    }

    #[test]
    fn storage_escape_matches_guava_html_escaper() {
        assert_eq!(
            sEscapeForStorage("A & B < C > D \"Q\" 'X'"),
            "A &amp; B &lt; C &gt; D &quot;Q&quot; &#39;X&#39;"
        );
    }

    #[test]
    fn edit_decode_removes_only_one_entity_layer() {
        assert_eq!(
            sUnescapeFromStorage("A &amp; B &lt; C &gt; D &quot;Q&quot; &#39;X&#39; &amp;lt;"),
            "A & B < C > D \"Q\" 'X' &lt;"
        );
    }

    #[test]
    fn unchanged_edit_round_trips_without_trimming_or_false_diff() {
        let sStoredTitle = "  A &amp; B &amp;lt; C  ";
        let sEditValue = sUnescapeFromStorage(sStoredTitle);

        assert_eq!(sEditValue, "  A & B &lt; C  ");
        assert_eq!(sEscapeForStorage(&sEditValue), sStoredTitle);
    }

    #[test]
    fn display_value_is_plain_text_and_must_be_escaped_by_the_view() {
        let sPlain = sPlainForDisplay(
            "A &amp; B &lt;b&gt; &quot;Q&quot; &#39;X&#39; &amp;lt;script&amp;gt;",
        );

        assert_eq!(sPlain, "A & B <b> \"Q\" 'X' &lt;script&gt;");
        let sHtml = html_escape::encode_text(&sPlain);
        assert!(!sHtml.contains("<b>"));
        assert!(!sHtml.contains("<script>"));
        assert!(!sHtml.contains("&amp;amp;"));
    }

    #[test]
    fn process_title_matches_java_string_util_golden() {
        let sInput = "one -- two --- three -- four-- five --six --";
        assert_eq!(
            sProcessTitleForLegacyView(sInput),
            "one&nbsp;&mdash; two --- three&nbsp;&mdash; four-- five --six --"
        );
        assert_eq!(
            sProcessTitlePlainForDisplay(sInput),
            "one\u{a0}— two --- three\u{a0}— four-- five --six --"
        );

        // java.lang.String.trim() does not remove NBSP; Rust str::trim does.
        assert_eq!(
            sProcessTitleForLegacyView("\u{a0} A -- B \u{a0}"),
            "\u{a0} A&nbsp;&mdash; B \u{a0}"
        );
    }

    #[test]
    fn make_title_matches_java_string_util_golden() {
        let sInput = "\"Test of \"quotes '' \"in quotes\" in title\"\"";
        assert_eq!(
            sMakeTitleForLegacyView(sInput),
            "&#171;Test of &#8222;quotes &quot; &#8222;in quotes&#8220; in title&#8220;&#187;"
        );
        assert_eq!(
            sMakeTitlePlainForDisplay(sInput),
            "«Test of „quotes \" „in quotes“ in title“»"
        );
    }

    #[test]
    fn regular_topic_pipeline_combines_make_process_and_empty_fallback() {
        assert_eq!(
            sTopicTitlePlainForDisplay("  &quot;LOR&quot; -- Rust  "),
            "«LOR»\u{a0}— Rust"
        );
        assert_eq!(sTopicTitlePlainForDisplay(" \t\r\n"), "Без заглавия");
    }

    #[test]
    fn comment_title_pipeline_decodes_one_layer_and_processes_title_tag() {
        assert_eq!(
            optCommentTitlePlainForDisplay("  A &amp; B &lt;b&gt; -- tail  "),
            Some("A & B <b>\u{a0}— tail".to_owned())
        );
        assert_eq!(
            optCommentTitlePlainForDisplay("&amp;lt;"),
            Some("&lt;".to_owned())
        );
        assert_eq!(optCommentTitlePlainForDisplay(" \t\r\n"), None);
    }

    #[test]
    fn askama_escapes_plain_display_title_in_text_and_attribute_contexts() {
        let sPlain = sPlainForDisplay(
            "A &amp; B &lt;b&gt; &quot;Q&quot; &#39;X&#39; &amp;lt;script&amp;gt;",
        );
        let sHtml = StPlainTitleTemplate { sTitle: &sPlain }
            .render()
            .expect("plain title template");

        assert!(sHtml.contains("A &#38; B &#60;b&#62; &#34;Q&#34; &#39;X&#39;"));
        assert!(!sHtml.contains("<b>"));
        assert!(!sHtml.contains("<script>"));
        assert!(!sHtml.contains("&#38;amp;"));
    }

    #[test]
    fn public_topic_templates_never_render_the_raw_storage_field() {
        let vecContracts = [
            (
                "topic.html",
                include_str!("../../templates/topic.html"),
                "topic.sTitlePlain()",
                "{{ topic.title }}",
            ),
            (
                "news_card.html",
                include_str!("../../templates/news_card.html"),
                "t.topic.sTitlePlain()",
                "{{ t.topic.title }}",
            ),
            (
                "main_page.html",
                include_str!("../../templates/main_page.html"),
                "t.sTitlePlain()",
                "{{ t.title }}",
            ),
            (
                "index.html",
                include_str!("../../templates/index.html"),
                "t.sTitlePlain()",
                "{{ t.title }}",
            ),
            (
                "group_topics.html",
                include_str!("../../templates/group_topics.html"),
                "t.topic.sTitlePlain()",
                "{{ t.topic.title }}",
            ),
            (
                "tag_page.html",
                include_str!("../../templates/tag_page.html"),
                "sTitlePlain()",
                "{{ t.topic.title }}",
            ),
            (
                "tracker.html",
                include_str!("../../templates/tracker.html"),
                "t.sTitlePlain()",
                "{{ t.stRow.title }}",
            ),
        ];

        for (sName, sTemplate, sPlainExpression, sRawExpression) in vecContracts {
            assert!(
                sTemplate.contains(sPlainExpression),
                "{sName} must use the plain-title accessor"
            );
            assert!(
                !sTemplate.contains(sRawExpression),
                "{sName} must not expose the raw stored title"
            );
            assert!(
                !sTemplate.contains(&format!("{sPlainExpression}|safe")),
                "{sName} must leave escaping to Askama"
            );
        }
    }

    #[test]
    fn public_comment_surfaces_use_the_plain_title_contract() {
        let sTopicTemplate = include_str!("../../templates/topic.html");
        assert!(sTopicTemplate.contains("c.item.optTitlePlain()"));
        assert!(!sTopicTemplate.contains("c.item.title"));
        assert!(!sTopicTemplate.contains("title|safe"));

        let sTopicsRoute = include_str!("../routes/topics.rs");
        assert!(sTopicsRoute.contains("title: stComment.optTitlePlain()"));

        let sLegacyRoute = include_str!("../routes/legacy.rs");
        assert!(sLegacyRoute.contains(".optTitlePlain()"));
        assert!(!sLegacyRoute.contains("encode_text(&stComment.title)"));

        let sCommentsRoute = include_str!("../routes/comments.rs");
        assert!(
            sCommentsRoute
                .contains("crate::domain::title::optCommentTitlePlainForDisplay(sStoredTitle)")
        );
        assert!(
            sCommentsRoute.contains(
                "crate::domain::title::optCommentTitlePlainForDisplay(&stComment.sTitle)"
            )
        );
    }

    #[test]
    fn manually_built_html_decodes_before_contextual_escaping() {
        let sApi = include_str!("../routes/api.rs");
        assert!(sApi.contains("let sSubjectPlain = stEvent.sSubjectPlain();"));
        assert!(sApi.contains("subj = html_escape::encode_text(&sSubjectPlain)"));

        let sUsers = include_str!("../routes/users.rs");
        assert!(sUsers.contains("let sTitlePlain = item.sTitlePlain();"));
        assert!(sUsers.contains("html_escape::encode_text(&sTitlePlain)"));

        let sComments = include_str!("../routes/comments.rs");
        assert!(sComments.contains("let sTopicTitlePlain = stTopic.sTitlePlain();"));
        assert!(sComments.contains("html_escape::encode_text(&sTopicTitlePlain)"));
    }

    #[test]
    fn java_length_counts_supplementary_characters_as_two_units() {
        assert_eq!(iJavaStringLength(&"😀".repeat(70)), 140);
        assert_eq!(iJavaStringLength(&"😀".repeat(71)), 142);
    }
}
