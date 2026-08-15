use once_cell::sync::Lazy;
use regex::Regex;

use crate::domain::markup::model::StMarkupUserDirectory;

use super::bSameSiteUrl;

const MAX_LORCODE_NESTING: usize = 256;

static TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\[\[?/?([a-z*]+)(?::[a-f0-9]+)?(=[^\]]+)?\]?\]").expect("LORCODE tag regex")
});

static VALID_URL_RE: Lazy<Regex> = Lazy::new(|| {
    // URLUtil.IsUrl.  Keeping validation separate from attribute escaping is
    // important: the original accepts several relaxed URI characters, but
    // never writes them to an attribute without escaping them first.
    Regex::new(
        r"(?ix)^(?:(?:(?:https?|ftp)://(?:(?:[0-9\p{L}.-]+\.[0-9\p{L}]+)|(?:\d+\.\d+\.\d+\.\d+))(?::[0-9]+)?(?:/\S*)?)|(?:mailto:[a-z0-9_+.-]+@[0-9a-z.-]+\.[a-z]+)|(?:news:[a-z0-9.-]+)|(?:(?:www|ftp)\.(?:(?:[0-9a-z.-]+\.[a-z]+(?::[0-9]+)?(?:/\S*)?)|(?:[a-z]+(?:/\S*)?))))$",
    )
    .expect("LOR URL validation regex")
});

static AUTO_URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:https?://|ftp://|www\.|ftp\.)[^\s<]+|mailto:\s?[a-z0-9+._-]+@[a-z0-9.-]+\.[a-z]+|news:(?:[a-z0-9_+]\.?)+",
    )
    .expect("LOR autolink regex")
});

#[derive(Debug, Clone, Copy)]
pub(super) enum EnCutMode<'a> {
    Comment,
    TopicExpanded,
    TopicMinimized(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnTag {
    Br,
    SoftBr,
    B,
    I,
    U,
    S,
    Em,
    Strong,
    Url,
    UrlWithParam,
    User,
    P,
    Div,
    Quote,
    List,
    Pre,
    Code,
    Inline,
    Cut,
    Li,
}

#[derive(Debug, Clone)]
enum EnNodeKind {
    Root,
    Text { sText: String, bCode: bool },
    Tag(EnTag),
}

#[derive(Debug, Clone)]
struct StNode {
    enKind: EnNodeKind,
    optParent: Option<usize>,
    sParameter: String,
    vecChildren: Vec<usize>,
}

impl StNode {
    fn stRoot() -> Self {
        Self {
            enKind: EnNodeKind::Root,
            optParent: None,
            sParameter: String::new(),
            vecChildren: Vec::new(),
        }
    }
}

struct CParser {
    vecNodes: Vec<StNode>,
    iCurrent: usize,
    bCode: bool,
    bFirstCode: bool,
}

impl CParser {
    fn stParse(sSource: &str) -> Self {
        let mut cParser = Self {
            vecNodes: vec![StNode::stRoot()],
            iCurrent: 0,
            bCode: false,
            bFirstCode: false,
        };
        let mut iPos = 0usize;

        for stCapture in TAG_RE.captures_iter(sSource) {
            let stWhole = stCapture.get(0).expect("whole LORCODE tag");
            let mut sBefore = &sSource[iPos..stWhole.start()];
            if cParser.bFirstCode {
                sBefore = sBefore
                    .strip_prefix('\n')
                    .or_else(|| sBefore.strip_prefix("\r\n"))
                    .unwrap_or(sBefore);
                cParser.bFirstCode = false;
            }
            cParser.vPushText(sBefore);

            let sWhole = stWhole.as_str();
            let sName = stCapture
                .get(1)
                .expect("LORCODE tag name")
                .as_str()
                .to_ascii_lowercase();
            let optTag = EnTag::optFromName(&sName);
            let bEscaped = sWhole.starts_with("[[") && sWhole.ends_with("]]");

            if bEscaped {
                if optTag.is_some() && !cParser.bCode {
                    cParser.vPushText(&sWhole[1..sWhole.len() - 1]);
                } else {
                    cParser.vPushText(sWhole);
                }
                iPos = stWhole.end();
                continue;
            }

            let Some(enTag) = optTag else {
                cParser.vPushText(sWhole);
                iPos = stWhole.end();
                continue;
            };
            let bClosing = sWhole.starts_with("[/") || sWhole.starts_with("[[/");
            let bCodeTag = matches!(enTag, EnTag::Code | EnTag::Inline);

            if sWhole.starts_with("[[") {
                cParser.vPushText("[");
            }

            if bClosing {
                if !cParser.bCode || bCodeTag {
                    cParser.vCloseTag(enTag);
                } else {
                    cParser.vPushText(sWhole);
                }
                if bCodeTag {
                    cParser.bCode = false;
                }
            } else if cParser.bCode && !bCodeTag {
                let mut sLiteral = sWhole;
                if sLiteral.starts_with("[[") {
                    sLiteral = &sLiteral[1..];
                }
                if sLiteral.ends_with("]]") {
                    sLiteral = &sLiteral[..sLiteral.len().saturating_sub(1)];
                }
                cParser.vPushText(sLiteral);
            } else {
                let sParameter = stCapture
                    .get(2)
                    .map(|stValue| stValue.as_str().trim_start_matches('='))
                    .unwrap_or_default();
                let enActualTag = if enTag == EnTag::Url && !sParameter.is_empty() {
                    EnTag::UrlWithParam
                } else {
                    enTag
                };
                if cParser.iDepth(cParser.iCurrent) >= MAX_LORCODE_NESTING {
                    cParser.vPushText(sWhole);
                } else {
                    if bCodeTag {
                        cParser.bCode = true;
                        cParser.bFirstCode = true;
                    }
                    cParser.iCurrent =
                        cParser.iPushTag(cParser.iCurrent, enActualTag, sParameter.to_owned());
                }
            }

            if sWhole.ends_with("]]") {
                cParser.vPushText("]");
            }
            iPos = stWhole.end();
        }

        // Parser.java only trims the first code newline when another tag is
        // encountered.  An unclosed code block therefore keeps it.
        cParser.vPushText(&sSource[iPos..]);
        cParser
    }

    fn iDepth(&self, mut iNode: usize) -> usize {
        let mut iDepth = 0usize;
        while let Some(iParent) = self.vecNodes[iNode].optParent {
            iDepth += 1;
            iNode = iParent;
        }
        iDepth
    }

    fn bAllows(&self, iNode: usize, sChild: &str) -> bool {
        match self.vecNodes[iNode].enKind {
            EnNodeKind::Root => bBlockLevel(sChild),
            EnNodeKind::Tag(enTag) => bTagAllows(enTag, sChild) && !self.bProhibited(iNode, sChild),
            EnNodeKind::Text { .. } => false,
        }
    }

    fn bProhibited(&self, iNode: usize, sChild: &str) -> bool {
        if matches!(self.vecNodes[iNode].enKind, EnNodeKind::Tag(EnTag::P))
            && matches!(sChild, "div" | "list" | "quote" | "cut")
        {
            return true;
        }
        self.vecNodes[iNode]
            .optParent
            .is_some_and(|iParent| self.bProhibited(iParent, sChild))
    }

    fn iAddTag(&mut self, iParent: usize, enTag: EnTag, sParameter: String) -> usize {
        let iNode = self.vecNodes.len();
        self.vecNodes.push(StNode {
            enKind: EnNodeKind::Tag(enTag),
            optParent: Some(iParent),
            sParameter,
            vecChildren: Vec::new(),
        });
        self.vecNodes[iParent].vecChildren.push(iNode);
        iNode
    }

    fn iPushTag(&mut self, mut iCurrent: usize, enTag: EnTag, sParameter: String) -> usize {
        loop {
            if self.bAllows(iCurrent, enTag.sName()) {
                let iNode = self.iAddTag(iCurrent, enTag, sParameter);
                return if enTag.bSelfClosing() {
                    iCurrent
                } else {
                    iNode
                };
            }

            if enTag == EnTag::SoftBr {
                return iCurrent;
            }

            let optImplicit = enTag.optImplicit();
            let bAtRoot = iCurrent == 0;
            let bCurrentBlock = self.bNodeBlockLevel(iCurrent);
            if bAtRoot || (bCurrentBlock && optImplicit.is_some()) {
                if matches!(self.vecNodes[iCurrent].enKind, EnNodeKind::Tag(EnTag::P)) {
                    iCurrent = self.vecNodes[iCurrent].optParent.unwrap_or(0);
                    continue;
                }
                if let Some(enImplicit) = optImplicit {
                    iCurrent = self.iPushTag(iCurrent, enImplicit, String::new());
                    continue;
                }
            }

            iCurrent = self.vecNodes[iCurrent].optParent.unwrap_or(0);
        }
    }

    fn bNodeBlockLevel(&self, iNode: usize) -> bool {
        matches!(
            self.vecNodes[iNode].enKind,
            EnNodeKind::Tag(enTag) if bBlockLevel(enTag.sName())
        )
    }

    fn vCloseTag(&mut self, enTag: EnTag) {
        let mut iCandidate = self.iCurrent;
        while iCandidate != 0 {
            if let EnNodeKind::Tag(enOpen) = self.vecNodes[iCandidate].enKind
                && (enOpen == enTag || (enTag == EnTag::Url && enOpen == EnTag::UrlWithParam))
            {
                self.iCurrent = self.vecNodes[iCandidate].optParent.unwrap_or(0);
                return;
            }
            iCandidate = self.vecNodes[iCandidate].optParent.unwrap_or(0);
        }
    }

    fn vPushText(&mut self, sText: &str) {
        if sText.trim().is_empty() && !self.bAllows(self.iCurrent, "text") {
            return;
        }

        while !self.bAllows(self.iCurrent, "text") {
            if self.bAllows(self.iCurrent, "p") {
                self.iCurrent = self.iAddTag(self.iCurrent, EnTag::P, String::new());
            } else if self.bAllows(self.iCurrent, "div") {
                self.iCurrent = self.iAddTag(self.iCurrent, EnTag::Div, String::new());
            } else {
                self.iCurrent = self.vecNodes[self.iCurrent].optParent.unwrap_or(0);
            }
        }

        let (bParagraph, bAllowParagraph, bKeepParagraphBreaks) =
            match self.vecNodes[self.iCurrent].enKind {
                EnNodeKind::Tag(enTag) => (
                    enTag == EnTag::P,
                    !matches!(enTag, EnTag::Pre | EnTag::Url | EnTag::User | EnTag::Code),
                    matches!(enTag, EnTag::Pre | EnTag::Code),
                ),
                _ => (false, true, false),
            };

        if bAllowParagraph && let Some((iStart, iEnd)) = optFirstParagraphBreak(sText) {
            let sHead = &sText[..iStart];
            let sTail = &sText[iEnd..];
            if !sHead.is_empty() {
                self.vAddTextNode(sHead);
            }
            if bParagraph {
                self.iCurrent = self.vecNodes[self.iCurrent].optParent.unwrap_or(0);
            }
            if !sTail.is_empty() {
                self.iCurrent = self.iAddTag(self.iCurrent, EnTag::P, " ".to_owned());
                self.vPushText(sTail);
            }
            return;
        }

        if bKeepParagraphBreaks {
            self.vAddTextNode(sText);
        } else {
            self.vAddTextNode(&sRemoveParagraphBreaks(sText));
        }
    }

    fn vAddTextNode(&mut self, sText: &str) {
        if sText.is_empty() {
            return;
        }
        let iNode = self.vecNodes.len();
        self.vecNodes.push(StNode {
            enKind: EnNodeKind::Text {
                sText: sText.to_owned(),
                bCode: self.bCode,
            },
            optParent: Some(self.iCurrent),
            sParameter: String::new(),
            vecChildren: Vec::new(),
        });
        self.vecNodes[self.iCurrent].vecChildren.push(iNode);
    }
}

impl EnTag {
    fn optFromName(sName: &str) -> Option<Self> {
        Some(match sName {
            "br" => Self::Br,
            "softbr" => Self::SoftBr,
            "b" => Self::B,
            "i" => Self::I,
            "u" => Self::U,
            "s" => Self::S,
            "em" => Self::Em,
            "strong" => Self::Strong,
            "url" => Self::Url,
            "user" => Self::User,
            "p" => Self::P,
            "div" => Self::Div,
            "quote" => Self::Quote,
            "list" => Self::List,
            "pre" => Self::Pre,
            "code" => Self::Code,
            "inline" => Self::Inline,
            "cut" => Self::Cut,
            "*" => Self::Li,
            _ => return None,
        })
    }

    fn sName(self) -> &'static str {
        match self {
            Self::Br => "br",
            Self::SoftBr => "softbr",
            Self::B => "b",
            Self::I => "i",
            Self::U => "u",
            Self::S => "s",
            Self::Em => "em",
            Self::Strong => "strong",
            Self::Url => "url",
            Self::UrlWithParam => "url2",
            Self::User => "user",
            Self::P => "p",
            Self::Div => "div",
            Self::Quote => "quote",
            Self::List => "list",
            Self::Pre => "pre",
            Self::Code => "code",
            Self::Inline => "inline",
            Self::Cut => "cut",
            Self::Li => "*",
        }
    }

    fn optImplicit(self) -> Option<Self> {
        match self {
            Self::Br
            | Self::SoftBr
            | Self::B
            | Self::I
            | Self::U
            | Self::S
            | Self::Em
            | Self::Strong
            | Self::Url
            | Self::UrlWithParam
            | Self::User
            | Self::Inline => Some(Self::P),
            Self::Quote | Self::List | Self::Pre | Self::Code | Self::Cut => Some(Self::Div),
            Self::Li => Some(Self::List),
            Self::P | Self::Div => None,
        }
    }

    fn bSelfClosing(self) -> bool {
        matches!(self, Self::Br | Self::SoftBr)
    }
}

fn bInline(sName: &str) -> bool {
    matches!(
        sName,
        "b" | "i"
            | "u"
            | "s"
            | "em"
            | "strong"
            | "url"
            | "url2"
            | "user"
            | "br"
            | "text"
            | "softbr"
            | "inline"
    )
}

fn bBlockLevel(sName: &str) -> bool {
    matches!(
        sName,
        "p" | "quote" | "list" | "pre" | "code" | "div" | "cut"
    )
}

fn bFlow(sName: &str) -> bool {
    bInline(sName) || bBlockLevel(sName)
}

fn bTagAllows(enTag: EnTag, sChild: &str) -> bool {
    match enTag {
        EnTag::Br | EnTag::SoftBr => false,
        EnTag::B
        | EnTag::I
        | EnTag::U
        | EnTag::S
        | EnTag::Em
        | EnTag::Strong
        | EnTag::Pre
        | EnTag::Code
        | EnTag::Inline => bInline(sChild),
        EnTag::Url | EnTag::User => sChild == "text",
        EnTag::UrlWithParam => matches!(sChild, "b" | "i" | "u" | "s" | "strong" | "text"),
        EnTag::P | EnTag::Li => bFlow(sChild),
        EnTag::Div | EnTag::Quote | EnTag::Cut => bBlockLevel(sChild),
        EnTag::List => matches!(sChild, "*" | "softbr"),
    }
}

fn optFirstParagraphBreak(sText: &str) -> Option<(usize, usize)> {
    let vecBytes = sText.as_bytes();
    let mut iAt = 0usize;
    while iAt < vecBytes.len() {
        let iStart = iAt;
        let mut iCount = 0usize;
        while iAt < vecBytes.len() {
            if vecBytes[iAt] == b'\n' {
                iAt += 1;
                iCount += 1;
            } else if vecBytes[iAt] == b'\r'
                && iAt + 1 < vecBytes.len()
                && vecBytes[iAt + 1] == b'\n'
            {
                iAt += 2;
                iCount += 1;
            } else {
                break;
            }
        }
        if iCount >= 2 {
            return Some((iStart, iAt));
        }
        if iAt == iStart {
            iAt += 1;
        }
    }
    None
}

fn sRemoveParagraphBreaks(sText: &str) -> String {
    let mut sOut = String::with_capacity(sText.len());
    let mut iAt = 0usize;
    while iAt < sText.len() {
        if let Some((iStart, iEnd)) = optFirstParagraphBreak(&sText[iAt..]) {
            sOut.push_str(&sText[iAt..iAt + iStart]);
            iAt += iEnd;
        } else {
            sOut.push_str(&sText[iAt..]);
            break;
        }
    }
    sOut
}

struct StRenderContext<'a> {
    bNofollow: bool,
    optSiteOrigin: Option<&'a str>,
    optUsers: Option<&'a StMarkupUserDirectory>,
    enCutMode: EnCutMode<'a>,
    iCutCount: usize,
    stTypo: StTypoChanger,
}

impl CParser {
    fn sRender<'a>(
        &self,
        bNofollow: bool,
        optSiteOrigin: Option<&'a str>,
        optUsers: Option<&'a StMarkupUserDirectory>,
        enCutMode: EnCutMode<'a>,
    ) -> String {
        let mut stContext = StRenderContext {
            bNofollow,
            optSiteOrigin,
            optUsers,
            enCutMode,
            iCutCount: 0,
            stTypo: StTypoChanger::default(),
        };
        self.sRenderChildren(0, &mut stContext)
    }

    fn sRenderChildren(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        let mut sOut = String::new();
        for &iChild in &self.vecNodes[iNode].vecChildren {
            sOut.push_str(&self.sRenderNode(iChild, stContext));
        }
        sOut
    }

    fn sRenderNode(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        let stNode = &self.vecNodes[iNode];
        match &stNode.enKind {
            EnNodeKind::Root => self.sRenderChildren(iNode, stContext),
            EnNodeKind::Text { sText, bCode } => {
                if *bCode {
                    sEscapeCode(sText)
                } else {
                    let bAutoLink = stNode
                        .optParent
                        .and_then(|iParent| match self.vecNodes[iParent].enKind {
                            EnNodeKind::Tag(enTag) => Some(matches!(
                                enTag,
                                EnTag::B
                                    | EnTag::I
                                    | EnTag::U
                                    | EnTag::S
                                    | EnTag::Em
                                    | EnTag::Strong
                                    | EnTag::P
                                    | EnTag::Quote
                                    | EnTag::Div
                                    | EnTag::Cut
                                    | EnTag::Pre
                                    | EnTag::Li
                            )),
                            _ => None,
                        })
                        .unwrap_or(false);
                    sFormatText(sText, bAutoLink, stContext)
                }
            }
            EnNodeKind::Tag(enTag) => self.sRenderTag(iNode, *enTag, stContext),
        }
    }

    fn sRenderTag(
        &self,
        iNode: usize,
        enTag: EnTag,
        stContext: &mut StRenderContext<'_>,
    ) -> String {
        let stNode = &self.vecNodes[iNode];
        match enTag {
            EnTag::Br => "<br>".to_owned(),
            EnTag::SoftBr => {
                let bParentAllowsBr = stNode
                    .optParent
                    .is_some_and(|iParent| self.bAllows(iParent, "br"));
                if bParentAllowsBr {
                    "<br>".to_owned()
                } else {
                    "\n".to_owned()
                }
            }
            EnTag::B => self.sRenderHtmlElement(iNode, "b", stContext),
            EnTag::I => self.sRenderHtmlElement(iNode, "i", stContext),
            EnTag::U => self.sRenderHtmlElement(iNode, "u", stContext),
            EnTag::S => self.sRenderHtmlElement(iNode, "s", stContext),
            EnTag::Em => self.sRenderHtmlElement(iNode, "em", stContext),
            EnTag::Strong => self.sRenderHtmlElement(iNode, "strong", stContext),
            EnTag::P => self.sRenderHtmlElement(iNode, "p", stContext),
            EnTag::Div => self.sRenderChildren(iNode, stContext),
            EnTag::Pre => self.sRenderHtmlElement(iNode, "pre", stContext),
            EnTag::Li => self.sRenderHtmlElement(iNode, "li", stContext),
            EnTag::Quote => self.sRenderQuote(iNode, stContext),
            EnTag::List => self.sRenderList(iNode, stContext),
            EnTag::Code => self.sRenderCode(iNode, stContext),
            EnTag::Inline => {
                if self.bEmptyNode(iNode) {
                    String::new()
                } else {
                    format!(
                        "<span class=\"code\"><code>{}</code></span>",
                        self.sRenderChildren(iNode, stContext)
                    )
                }
            }
            EnTag::Cut => self.sRenderCut(iNode, stContext),
            EnTag::Url => self.sRenderUrl(iNode, stContext),
            EnTag::UrlWithParam => self.sRenderUrlWithParam(iNode, stContext),
            EnTag::User => self.sRenderUser(iNode, stContext),
        }
    }

    fn sRenderUser(&self, iNode: usize, stContext: &StRenderContext<'_>) -> String {
        let Some(sInputNick) = self.optFirstText(iNode).map(str::trim) else {
            return String::new();
        };
        let sEscapedInputNick = sStrangeEscapeHtml(sInputNick);
        let Some(stUsers) = stContext.optUsers else {
            // Exact null-UserService branch used by isolated Java formatter
            // tests and retained for pure utility callers.
            return sEscapedInputNick;
        };
        let Some(stUser) = stUsers.optFind(sInputNick) else {
            return format!(" <s>{sEscapedInputNick}</s>");
        };
        let sOrigin = stContext
            .optSiteOrigin
            .unwrap_or_default()
            .trim_end_matches('/');
        let sHref = format!("{sOrigin}/people/{}/profile", stUser.sCanonicalNick);
        let sLink = format!(
            "<a style=\"text-decoration: none\" href=\"{}\">{sEscapedInputNick}</a>",
            sEscapeAttribute(&sHref)
        );
        if stUser.bBlocked {
            format!(
                " <span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\"><s>{sLink}</s></span>"
            )
        } else {
            format!(
                " <span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\">{sLink}</span>"
            )
        }
    }

    fn sRenderHtmlElement(
        &self,
        iNode: usize,
        sElement: &str,
        stContext: &mut StRenderContext<'_>,
    ) -> String {
        if self.vecNodes[iNode].vecChildren.is_empty() {
            String::new()
        } else {
            format!(
                "<{sElement}>{}</{sElement}>",
                self.sRenderChildren(iNode, stContext)
            )
        }
    }

    fn bEmptyNode(&self, iNode: usize) -> bool {
        let vecChildren = &self.vecNodes[iNode].vecChildren;
        if vecChildren.is_empty() {
            return true;
        }
        if vecChildren.len() == 1
            && let EnNodeKind::Text { sText, .. } = &self.vecNodes[vecChildren[0]].enKind
        {
            return sText.trim().is_empty();
        }
        false
    }

    fn sRenderQuote(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        if self.bEmptyNode(iNode) {
            return String::new();
        }
        let stNode = &self.vecNodes[iNode];
        let bOnlyNestedQuote = stNode.vecChildren.len() == 1
            && matches!(
                self.vecNodes[stNode.vecChildren[0]].enKind,
                EnNodeKind::Tag(EnTag::Quote)
            );
        let sChildren = self.sRenderChildren(iNode, stContext);
        let sParameter = stNode.sParameter.trim();
        if !sParameter.is_empty() {
            format!(
                "<blockquote><p><cite>{}</cite></p>{sChildren}</blockquote>",
                sSimpleFormat(&sParameter.replace('"', ""))
            )
        } else if bOnlyNestedQuote {
            sChildren
        } else {
            format!("<blockquote>{sChildren}</blockquote>")
        }
    }

    fn sRenderList(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        if self.vecNodes[iNode].vecChildren.is_empty() {
            return String::new();
        }
        let sChildren = self.sRenderChildren(iNode, stContext);
        let sParameter = self.vecNodes[iNode].sParameter.trim().replace('"', "");
        if matches!(sParameter.as_str(), "A" | "a" | "I" | "i" | "1") {
            format!("<ol type=\"{sParameter}\">{sChildren}</ol>")
        } else {
            format!("<ul>{sChildren}</ul>")
        }
    }

    fn sRenderCode(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        if self.bEmptyNode(iNode) {
            return String::new();
        }
        let sClass = sCodeClass(self.vecNodes[iNode].sParameter.trim());
        format!(
            "<div class=\"code\"><pre class=\"{sClass}\"><code>{}</code></pre></div>",
            self.sRenderChildren(iNode, stContext)
        )
    }

    fn sRenderCut(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        if self.bEmptyNode(iNode) {
            return String::new();
        }
        match stContext.enCutMode {
            EnCutMode::Comment => self.sRenderChildren(iNode, stContext),
            EnCutMode::TopicExpanded => {
                let iCut = stContext.iCutCount;
                stContext.iCutCount += 1;
                format!(
                    "<div id=\"cut{iCut}\">{}</div>",
                    self.sRenderChildren(iNode, stContext)
                )
            }
            EnCutMode::TopicMinimized(sCanonicalUrl) => {
                let iCut = stContext.iCutCount;
                stContext.iCutCount += 1;
                let sHref = format!("{sCanonicalUrl}#cut{iCut}");
                let sLabel = if self.vecNodes[iNode].sParameter.trim().is_empty() {
                    "читать дальше...".to_owned()
                } else {
                    sSimpleFormat(&self.vecNodes[iNode].sParameter.trim().replace('"', ""))
                };
                format!(
                    "<p>( <a href=\"{}\">{sLabel}</a> )</p>",
                    sEscapeAttribute(&sHref)
                )
            }
        }
    }

    fn optFirstText(&self, iNode: usize) -> Option<&str> {
        let iChild = *self.vecNodes[iNode].vecChildren.first()?;
        match &self.vecNodes[iChild].enKind {
            EnNodeKind::Text { sText, .. } => Some(sText),
            _ => None,
        }
    }

    fn sRenderUrl(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        let Some(sText) = self.optFirstText(iNode) else {
            return String::new();
        };
        let sUrl = sText.trim();
        let sHref = sFixUrl(sUrl);
        let sLinkText = if sUrl.is_empty() { &sHref } else { sUrl };
        if !bValidUrl(&sHref) {
            return format!("<s>{}</s>", sStrangeEscapeHtml(sUrl));
        }
        sRenderLink(
            &sHref,
            &sSimpleFormat(sLinkText),
            stContext.bNofollow,
            stContext.optSiteOrigin,
        )
    }

    fn sRenderUrlWithParam(&self, iNode: usize, stContext: &mut StRenderContext<'_>) -> String {
        let mut sUrl = self.vecNodes[iNode].sParameter.trim().to_owned();
        if let Some(sWithout) = sUrl.strip_prefix('"') {
            sUrl = sWithout.to_owned();
            if sUrl.ends_with('"') {
                sUrl.pop();
            }
        } else if let Some(sWithout) = sUrl.strip_prefix('\'') {
            sUrl = sWithout.to_owned();
            if sUrl.ends_with('\'') {
                sUrl.pop();
            }
        }
        let sHref = sFixUrl(&sUrl);
        let bEmpty = self.vecNodes[iNode].vecChildren.is_empty()
            || (self.vecNodes[iNode].vecChildren.len() == 1
                && self
                    .optFirstText(iNode)
                    .is_some_and(|sText| sText.trim().is_empty()));

        if !bValidUrl(&sHref) {
            let sBody = if bEmpty {
                sStrangeEscapeHtml(&sUrl)
            } else {
                self.sRenderChildren(iNode, stContext)
            };
            return format!("<s title=\"{}\">{sBody}</s>", sEscapeAttribute(&sHref));
        }

        if bEmpty {
            return format!(
                "<a href=\"{}\">{}</a>",
                sEscapeAttribute(&sHref),
                sStrangeEscapeHtml(&sHref)
            );
        }

        let mut sBody = self.sRenderChildren(iNode, stContext);
        if self.sRenderOgChildren(iNode).chars().count() <= 3 {
            let sHost = optShortHost(&sHref).unwrap_or_else(|| "---".to_owned());
            sBody.push_str(" (");
            sBody.push_str(&sStrangeEscapeHtml(&sHost));
            sBody.push(')');
        }

        // UrlWithParamTag intentionally does not consult RootNode.nofollow in
        // the original.  Preserve that observable quirk here.
        format!("<a href=\"{}\">{sBody}</a>", sEscapeAttribute(&sHref))
    }

    fn sRenderOgChildren(&self, iNode: usize) -> String {
        let mut vecParts = Vec::new();
        for &iChild in &self.vecNodes[iNode].vecChildren {
            let sPart = self.sRenderOg(iChild);
            if !sPart.is_empty() {
                vecParts.push(sPart);
            }
        }
        vecParts.join(" ").trim().to_owned()
    }

    fn sRenderOg(&self, iNode: usize) -> String {
        match &self.vecNodes[iNode].enKind {
            EnNodeKind::Text { sText, .. } => sText.clone(),
            EnNodeKind::Tag(EnTag::Quote) => {
                if self.vecNodes[iNode].vecChildren.is_empty() {
                    String::new()
                } else {
                    format!("«{}»", self.sRenderOgChildren(iNode))
                }
            }
            EnNodeKind::Root | EnNodeKind::Tag(_) => self.sRenderOgChildren(iNode),
        }
    }
}

pub(super) fn render<'a>(
    sSource: &str,
    bUserLineBreak: bool,
    bNofollow: bool,
    optSiteOrigin: Option<&'a str>,
    optUsers: Option<&'a StMarkupUserDirectory>,
    enCutMode: EnCutMode<'a>,
) -> String {
    let sPrepared = sPrepare(sSource, if bUserLineBreak { "[br]" } else { "\n" });
    CParser::stParse(&sPrepared).sRender(bNofollow, optSiteOrigin, optUsers, enCutMode)
}

fn sPrepare(sText: &str, sNewLine: &str) -> String {
    let sNormalized = sText.replace("\r\n", "\n");
    let mut vecLines: Vec<&str> = sNormalized.split('\n').collect();
    while vecLines.last().is_some_and(|sLine| sLine.is_empty()) {
        vecLines.pop();
    }
    if vecLines.is_empty() {
        return String::new();
    }

    let mut sOut = String::new();
    let mut iQuoteDepth = 0usize;
    let mut bCode = false;
    for (iLine, sLine) in vecLines.iter().enumerate() {
        let bLast = iLine + 1 == vecLines.len();
        if sLine.is_empty() {
            if iQuoteDepth > 0 {
                for _ in 0..iQuoteDepth {
                    sOut.push_str("[/quote]");
                }
                iQuoteDepth = 0;
            } else if bCode {
                sOut.push('\n');
            } else {
                sOut.push_str(sNewLine);
            }
            continue;
        }

        let iLineQuoteDepth = if bCode {
            0
        } else {
            sLine.bytes().take_while(|bByte| *bByte == b'>').count()
        };
        if iLineQuoteDepth > 0 {
            if iQuoteDepth == 0 {
                for _ in 0..iLineQuoteDepth {
                    sOut.push_str("[quote]");
                }
            } else if iLineQuoteDepth < iQuoteDepth {
                for _ in 0..(iQuoteDepth - iLineQuoteDepth) {
                    sOut.push_str("[/quote]");
                }
            } else if iLineQuoteDepth > iQuoteDepth {
                for _ in 0..(iLineQuoteDepth - iQuoteDepth) {
                    sOut.push_str("[quote]");
                }
            }
            iQuoteDepth = iLineQuoteDepth;
            sOut.push_str(&sEscapeCodeTags(&sLine[iLineQuoteDepth..]));
            if !bLast {
                sOut.push_str("[br]");
            }
            continue;
        }

        if iQuoteDepth > 0 {
            for _ in 0..iQuoteDepth {
                sOut.push_str("[/quote]");
            }
            iQuoteDepth = 0;
        }

        if bContainsCodeTag(sLine, false) {
            bCode = true;
        }
        if bContainsCodeTag(sLine, true) {
            bCode = false;
        }
        sOut.push_str(sLine);
        if !bLast {
            if bCode {
                sOut.push('\n');
            } else if !sLine.ends_with("[quote]") {
                sOut.push_str(sNewLine);
            }
        }
    }
    for _ in 0..iQuoteDepth {
        sOut.push_str("[/quote]");
    }
    sOut
}

fn bContainsCodeTag(sLine: &str, bClosing: bool) -> bool {
    TAG_RE.captures_iter(sLine).any(|stCapture| {
        let stWhole = stCapture.get(0).expect("code tag");
        let sWhole = stWhole.as_str();
        let sName = stCapture.get(1).expect("code tag name").as_str();
        let bEscaped = sWhole.starts_with("[[") && sWhole.ends_with("]]");
        let bIsClosing = sWhole.starts_with("[/") || sWhole.starts_with("[[/");
        !bEscaped && sName.eq_ignore_ascii_case("code") && bIsClosing == bClosing
    })
}

fn sEscapeCodeTags(sText: &str) -> String {
    let mut sOut = String::with_capacity(sText.len());
    let mut iPos = 0usize;
    for stCapture in TAG_RE.captures_iter(sText) {
        let stWhole = stCapture.get(0).expect("quoted code tag");
        let sName = stCapture.get(1).expect("quoted code tag name").as_str();
        let bAlreadyEscaped =
            stWhole.start() > 0 && sText.as_bytes().get(stWhole.start() - 1) == Some(&b'[');
        if !bAlreadyEscaped && sName.eq_ignore_ascii_case("code") {
            sOut.push_str(&sText[iPos..stWhole.start()]);
            sOut.push('[');
            sOut.push_str(stWhole.as_str());
            sOut.push(']');
            iPos = stWhole.end();
        }
    }
    sOut.push_str(&sText[iPos..]);
    sOut
}

fn sCodeClass(sLanguage: &str) -> &'static str {
    match sLanguage.trim().to_ascii_lowercase().as_str() {
        "abnf" => "language-abnf",
        "ada" => "language-ada",
        "asm" | "asm-x86" => "language-x86asm",
        "asm-arm" => "language-armasm",
        "asm-avr" => "language-avrasm",
        "asm-mips" => "language-mipsasm",
        "awk" => "language-awk",
        "bas" | "basic" => "language-basic",
        "bash" | "shell" => "language-bash",
        "bnf" => "language-bnf",
        "brainfuck" => "language-brainfuck",
        "c" => "language-c",
        "c#" => "language-csharp",
        "c++" | "cc" | "cpp" | "cxx" => "language-cpp",
        "clojure" => "language-clojure",
        "cmake" => "language-cmake",
        "coffeescript" => "language-coffeescript",
        "cs" => "language-cs",
        "css" => "language-css",
        "d" => "language-d",
        "delphi" | "pas" | "pascal" => "language-delphi",
        "diff" | "patch" => "language-diff",
        "ebnf" => "language-ebnf",
        "erlang" => "language-erlang",
        "f#" | "fs" => "language-fsharp",
        "fortran" => "language-fortran",
        "go" => "language-go",
        "haskell" => "language-haskell",
        "html" => "language-html",
        "ini" => "language-ini",
        "java" => "language-java",
        "javascript" | "js" => "language-javascript",
        "jl" | "julia" => "language-julia",
        "json" => "language-json",
        "lisp" => "language-lisp",
        "llvm" => "language-llvm",
        "lua" => "language-lua",
        "makefile" => "language-makefile",
        "md" | "markdown" => "language-markdown",
        "nim" => "language-nim",
        "nix" => "language-nix",
        "ocaml" => "language-ocaml",
        "objc" | "objectivec" => "language-objectivec",
        "perl" => "language-perl",
        "php" => "language-php",
        "plain" => "no-highlight",
        "py" | "python" => "language-python",
        "rb" | "ruby" => "language-ruby",
        "rs" | "rust" => "language-rust",
        "scala" => "language-scala",
        "scheme" => "language-scheme",
        "smalltalk" => "language-smalltalk",
        "sql" => "language-sql",
        "tcl" => "language-tcl",
        "tex" => "language-latex",
        "ts" | "typescript" => "language-typescript",
        "vala" => "language-vala",
        "vim" => "language-vim",
        "wasm" => "language-wasm",
        "xml" => "language-xml",
        "yaml" => "language-yaml",
        _ => "no-highlight",
    }
}

fn sFormatText(sText: &str, bAutoLink: bool, stContext: &mut StRenderContext<'_>) -> String {
    if !bAutoLink {
        let sChanged = stContext.stTypo.sFormat(sText);
        return sSimpleFormat(&sChanged);
    }

    let mut sOut = String::new();
    let mut iPos = 0usize;
    for stMatch in AUTO_URL_RE.find_iter(sText) {
        if stMatch.start() > 0 {
            let chBefore = sText[..stMatch.start()].chars().next_back();
            if chBefore.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '.' | '/')) {
                continue;
            }
        }
        sOut.push_str(&sFormatTypographicFragment(
            &sText[iPos..stMatch.start()],
            &mut stContext.stTypo,
        ));
        let sCandidate = stMatch.as_str();
        let (sCandidateUrl, sSuffix) = splitAutoUrlSuffix(sCandidate);
        let sHref = sFixUrl(sCandidateUrl);
        if bValidAutoUrl(&sHref) {
            let sBody = sSimpleFormat(&sHref);
            sOut.push_str(&sRenderLink(
                &sHref,
                &sBody,
                stContext.bNofollow,
                stContext.optSiteOrigin,
            ));
        } else {
            sOut.push_str(&sFormatTypographicFragment(
                sCandidateUrl,
                &mut stContext.stTypo,
            ));
        }
        sOut.push_str(&sFormatTypographicFragment(sSuffix, &mut stContext.stTypo));
        iPos = stMatch.end();
    }
    sOut.push_str(&sFormatTypographicFragment(
        &sText[iPos..],
        &mut stContext.stTypo,
    ));
    sOut.replace(" -- ", "&nbsp;&mdash; ")
}

fn sFormatTypographicFragment(sText: &str, stTypo: &mut StTypoChanger) -> String {
    stTypo.sFormat(&sStrangeEscapeHtml(sText))
}

fn sRenderLink(sHref: &str, sBody: &str, bNofollow: bool, optSiteOrigin: Option<&str>) -> String {
    let sRel = if bNofollow && !bSameSiteUrl(sHref, optSiteOrigin) {
        " rel=\"nofollow\""
    } else {
        ""
    };
    format!("<a href=\"{}\"{sRel}>{sBody}</a>", sEscapeAttribute(sHref))
}

fn sFixUrl(sUrl: &str) -> String {
    let sTrimmed = sUrl.trim();
    if sTrimmed.to_ascii_lowercase().starts_with("www.") {
        format!("http://{sTrimmed}")
    } else if sTrimmed.to_ascii_lowercase().starts_with("ftp.") {
        format!("ftp://{sTrimmed}")
    } else {
        sTrimmed.to_owned()
    }
}

fn bValidUrl(sUrl: &str) -> bool {
    VALID_URL_RE.is_match(sUrl)
}

fn bValidAutoUrl(sUrl: &str) -> bool {
    if bValidUrl(sUrl) {
        return true;
    }
    reqwest::Url::parse(sUrl).is_ok_and(|stUrl| {
        matches!(stUrl.scheme(), "http" | "https" | "ftp") && stUrl.host_str().is_some()
    })
}

fn splitAutoUrlSuffix(sCandidate: &str) -> (&str, &str) {
    let mut iEnd = sCandidate.len();
    let iOpeningParens = sCandidate.bytes().filter(|bByte| *bByte == b'(').count();
    let mut iClosingParens = sCandidate.bytes().filter(|bByte| *bByte == b')').count();
    while let Some(chLast) = sCandidate[..iEnd].chars().next_back() {
        let bTrim = matches!(chLast, '.' | ',' | ';' | '!' | '?')
            || (chLast == ')' && iClosingParens > iOpeningParens);
        if !bTrim {
            break;
        }
        if chLast == ')' {
            iClosingParens = iClosingParens.saturating_sub(1);
        }
        iEnd -= chLast.len_utf8();
    }
    (&sCandidate[..iEnd], &sCandidate[iEnd..])
}

fn optShortHost(sUrl: &str) -> Option<String> {
    let stUrl = reqwest::Url::parse(sUrl).ok()?;
    let sHost = stUrl.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    psl::domain(sHost.as_bytes())
        .and_then(|stDomain| std::str::from_utf8(stDomain.as_bytes()).ok())
        .map(str::to_owned)
}

fn sSimpleFormat(sText: &str) -> String {
    sStrangeEscapeHtml(sText).replace(" -- ", "&nbsp;&mdash; ")
}

fn sEscapeCode(sText: &str) -> String {
    let mut sOut = String::with_capacity(sText.len());
    for ch in sText.chars() {
        match ch {
            '&' => sOut.push_str("&amp;"),
            '<' => sOut.push_str("&lt;"),
            '>' => sOut.push_str("&gt;"),
            '"' => sOut.push_str("&quot;"),
            '\'' => sOut.push_str("&#39;"),
            _ => sOut.push(ch),
        }
    }
    sOut
}

fn sEscapeAttribute(sText: &str) -> String {
    html_escape::encode_double_quoted_attribute(sText).into_owned()
}

fn sStrangeEscapeHtml(sText: &str) -> String {
    let mut sOut = String::with_capacity(sText.len());
    let mut iAt = 0usize;
    while iAt < sText.len() {
        let sTail = &sText[iAt..];
        let ch = sTail.chars().next().expect("valid character boundary");
        match ch {
            '<' => sOut.push_str("&lt;"),
            '>' => sOut.push_str("&gt;"),
            '"' => sOut.push_str("&quot;"),
            '&' => {
                if let Some(iLength) = optPreservedEntityLength(sTail) {
                    sOut.push_str(&sTail[..iLength]);
                    iAt += iLength;
                    continue;
                }
                sOut.push_str("&amp;");
            }
            _ => sOut.push(ch),
        }
        iAt += ch.len_utf8();
    }
    sOut
}

fn optPreservedEntityLength(sText: &str) -> Option<usize> {
    let sAfterAmp = sText.strip_prefix('&')?;
    let iSemicolon = sAfterAmp.find(';')?;
    let sEntity = &sAfterAmp[..iSemicolon];
    let bNumeric = sEntity.strip_prefix('#').is_some_and(|sDigits| {
        (2..=5).contains(&sDigits.len())
            && !sDigits.starts_with('0')
            && sDigits.bytes().all(|bByte| bByte.is_ascii_digit())
    });
    let bNamed = (1..=8).contains(&sEntity.len())
        && sEntity
            .bytes()
            .all(|bByte| bByte.is_ascii_alphanumeric() || bByte == b'_');
    (bNumeric || bNamed).then_some(iSemicolon + 2)
}

#[derive(Default)]
struct StTypoChanger {
    iQuoteDepth: usize,
    vecPrevious: Vec<char>,
}

impl StTypoChanger {
    fn sFormat(&mut self, sInput: &str) -> String {
        let sDecoded = sInput.replace("&quot;", "\"");
        let mut vecChars: Vec<char> = sDecoded.chars().collect();
        for iAt in 0..vecChars.len() {
            if vecChars[iAt] != '"' {
                continue;
            }
            if self.bClosing(&vecChars, iAt) && self.iQuoteDepth > 0 {
                vecChars[iAt] = if self.iQuoteDepth == 1 { '»' } else { '“' };
                self.iQuoteDepth -= 1;
            } else if bQuoteOpening(&vecChars, iAt) {
                vecChars[iAt] = if self.iQuoteDepth == 0 { '«' } else { '„' };
                self.iQuoteDepth += 1;
            }
        }
        self.vecPrevious.clone_from(&vecChars);

        let mut sOut = String::new();
        let mut iAt = 0usize;
        while iAt < vecChars.len() {
            if iAt + 1 < vecChars.len() && vecChars[iAt] == '\'' && vecChars[iAt + 1] == '\'' {
                sOut.push_str("&quot;");
                iAt += 2;
                continue;
            }
            match vecChars[iAt] {
                '"' => sOut.push_str("&quot;"),
                '«' => sOut.push_str("&#171;"),
                '»' => sOut.push_str("&#187;"),
                '„' => sOut.push_str("&#8222;"),
                '“' => sOut.push_str("&#8220;"),
                ch => sOut.push(ch),
            }
            iAt += 1;
        }
        sOut
    }

    fn bClosing(&self, vecChars: &[char], iAt: usize) -> bool {
        if iAt == 0 && self.vecPrevious.is_empty() {
            return false;
        }
        if iAt + 1 == vecChars.len() {
            return true;
        }
        let chAfter = chLastNonQuote(vecChars, iAt).unwrap_or(vecChars[iAt]);
        let chBefore = if iAt == 0 {
            chFirstNonQuoteBeforeEnd(&self.vecPrevious).unwrap_or('\0')
        } else {
            chFirstNonQuote(vecChars, iAt).unwrap_or(vecChars[iAt])
        };
        !bQuoteChar(chBefore) && !chAfter.is_alphanumeric()
    }
}

fn bQuoteOpening(vecChars: &[char], iAt: usize) -> bool {
    if iAt + 1 == vecChars.len() {
        return false;
    }
    let chBefore = if iAt == 0 {
        '\0'
    } else {
        chFirstNonQuote(vecChars, iAt).unwrap_or(vecChars[0])
    };
    let chAfter = chLastNonQuote(vecChars, iAt).unwrap_or(*vecChars.last().unwrap_or(&'\0'));
    !(chAfter.is_whitespace() || bPunctuation(chAfter) || chBefore.is_alphanumeric())
}

fn chFirstNonQuote(vecChars: &[char], iAt: usize) -> Option<char> {
    vecChars[..iAt]
        .iter()
        .rev()
        .copied()
        .find(|ch| !bQuoteChar(*ch))
}

fn chFirstNonQuoteBeforeEnd(vecChars: &[char]) -> Option<char> {
    vecChars.iter().rev().copied().find(|ch| !bQuoteChar(*ch))
}

fn chLastNonQuote(vecChars: &[char], iAt: usize) -> Option<char> {
    vecChars[iAt + 1..]
        .iter()
        .copied()
        .find(|ch| !bQuoteChar(*ch))
}

fn bQuoteChar(ch: char) -> bool {
    matches!(ch, '"' | '«' | '»' | '„' | '“')
}

fn bPunctuation(ch: char) -> bool {
    matches!(ch, '.' | ',' | ':' | ';' | '-' | '!' | '?' | '(' | ')')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sRender(sSource: &str) -> String {
        render(sSource, false, false, None, None, EnCutMode::Comment)
    }

    #[test]
    fn java_simple_parser_contract() {
        assert_eq!(sRender("[br]"), "<p><br></p>");
        assert_eq!(sRender("[b]hello[/b]"), "<p><b>hello</b></p>");
        assert_eq!(sRender("[i]hello[/i]"), "<p><i>hello</i></p>");
        assert_eq!(sRender("[s]hello[/s]"), "<p><s>hello</s></p>");
        assert_eq!(
            sRender("[strong]hello[/strong]"),
            "<p><strong>hello</strong></p>"
        );
        assert_eq!(
            sRender("[quote=maxcom]hello[/quote]"),
            "<blockquote><p><cite>maxcom</cite></p><p>hello</p></blockquote>"
        );
        assert_eq!(sRender("[quote][/quote]"), "");
        assert_eq!(
            sRender("[EM]em[/EM] [u]under[/u] [inline]a[b]=c[/inline]"),
            "<p><em>em</em> <u>under</u> <span class=\"code\"><code>a[b]=c</code></span></p>"
        );
        assert_eq!(
            sRender("[pre]line 1\n\nline 2[/pre]"),
            "<pre>line 1\n\nline 2</pre>"
        );
        // There is no standalone [spoiler] tag in DefaultParserParameters;
        // LOR's spoiler is [cut]. Unknown tags remain authored text.
        assert_eq!(
            sRender("[spoiler]secret[/spoiler]"),
            "<p>[spoiler]secret[/spoiler]</p>"
        );
        assert_eq!(sRender("[user]maxcom[/user]"), "<p>maxcom</p>");
        assert_eq!(
            sRender("This is \"local [u]buffer[/u]\" test"),
            "<p>This is &#171;local <u>buffer</u>&#187; test</p>"
        );
    }

    #[test]
    fn java_lists_and_paragraph_recovery_contract() {
        assert_eq!(
            sRender("[list][*]one[*]two[*]three[/list]"),
            "<ul><li>one</li><li>two</li><li>three</li></ul>"
        );
        assert_eq!(
            sRender("[list=A][*]one[*]two[/list]"),
            "<ol type=\"A\"><li>one</li><li>two</li></ol>"
        );
        assert_eq!(
            sRender("[list]0[*]1[*]2[/list]"),
            "<p>0</p><ul><li>1</li><li>2</li></ul>"
        );
        assert_eq!(
            sRender("test\ntest1\n\ntest2"),
            "<p>test\ntest1</p><p>test2</p>"
        );
    }

    #[test]
    fn java_code_and_escaping_contract() {
        assert_eq!(
            sRender("[code=cxx]\n#include <stdio.h>[/code]"),
            "<div class=\"code\"><pre class=\"language-cpp\"><code>#include &lt;stdio.h&gt;</code></pre></div>"
        );
        assert_eq!(
            sRender("[code][list][*]one[/list] https://evil.test[/code]"),
            "<div class=\"code\"><pre class=\"no-highlight\"><code>[list][*]one[/list] https://evil.test</code></pre></div>"
        );
        assert_eq!(sRender("[[b]]"), "<p>[b]</p>");
        assert_eq!(
            sRender("[code][[b]][/code][[b]]"),
            "<div class=\"code\"><pre class=\"no-highlight\"><code>[[b]]</code></pre></div><p>[b]</p>"
        );
        assert_eq!(
            sRender("[code]\"code&code\"[/code]"),
            "<div class=\"code\"><pre class=\"no-highlight\"><code>&quot;code&amp;code&quot;</code></pre></div>"
        );
        assert_eq!(
            sRender("<script>alert(1)</script>"),
            "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
    }

    #[test]
    fn urls_are_validated_escaped_and_apply_java_nofollow_policy() {
        let sExternal = render(
            "[url]https://outside.example/x[/url] https://www.linux.org.ru/forum/",
            false,
            true,
            Some("https://www.linux.org.ru"),
            None,
            EnCutMode::Comment,
        );
        assert!(
            sExternal.contains(
                "href=\"https://outside.example/x\" rel=\"nofollow\">https://outside.example/x"
            ),
            "{sExternal}"
        );
        assert!(
            sExternal.contains(
                "href=\"https://www.linux.org.ru/forum/\">https://www.linux.org.ru/forum/"
            ),
            "{sExternal}"
        );

        let sParameterized = render(
            "[url=https://outside.example.com/x]go[/url]",
            false,
            true,
            Some("https://www.linux.org.ru"),
            None,
            EnCutMode::Comment,
        );
        assert_eq!(
            sParameterized,
            "<p><a href=\"https://outside.example.com/x\">go (example.com)</a></p>"
        );
        assert_eq!(
            sRender("[url=javascript:alert(1)]bad[/url]"),
            "<p><s title=\"javascript:alert(1)\">bad</s></p>"
        );
        assert_eq!(
            sRender("[url=http://tts.com/\"><b>a</b>]usrl[/url]"),
            "<p><a href=\"http://tts.com/&quot;&gt;&lt;b&gt;a&lt;/b&gt;\">usrl</a></p>"
        );

        assert_eq!(
            sRender("See https://example.com/path, then ftp://u:p@example.org/file!"),
            "<p>See <a href=\"https://example.com/path\">https://example.com/path</a>, then <a href=\"ftp://u:p@example.org/file\">ftp://u:p@example.org/file</a>!</p>"
        );
        assert_eq!(
            sRender("(http://ru.wikipedia.org/wiki/Blah_(blah))"),
            "<p>(<a href=\"http://ru.wikipedia.org/wiki/Blah_(blah)\">http://ru.wikipedia.org/wiki/Blah_(blah)</a>)</p>"
        );
    }

    #[test]
    fn quote_prefix_modes_and_cut_context_match_java_contract() {
        assert_eq!(
            render(">one\n>two", false, false, None, None, EnCutMode::Comment),
            "<blockquote><p>one<br>two</p></blockquote>"
        );
        assert_eq!(
            render("one\ntwo", true, false, None, None, EnCutMode::Comment),
            "<p>one<br>two</p>"
        );
        assert_eq!(
            render(
                "before[cut=more]hidden[/cut]after",
                false,
                false,
                None,
                None,
                EnCutMode::TopicMinimized("/forum/g/1"),
            ),
            "<p>before</p><p>( <a href=\"/forum/g/1#cut0\">more</a> )</p><p>after</p>"
        );
        assert_eq!(
            render(
                "[code][cut]literal[/cut][/code]",
                false,
                false,
                None,
                None,
                EnCutMode::TopicMinimized("/forum/g/1"),
            ),
            "<div class=\"code\"><pre class=\"no-highlight\"><code>[cut]literal[/cut]</code></pre></div>"
        );
    }

    #[test]
    fn java_member_tag_existing_blocked_missing_and_null_service_contract() {
        use crate::domain::markup::model::{StMarkupUser, StMarkupUserDirectory};

        let stUsers = StMarkupUserDirectory::stFromUsers(vec![
            StMarkupUser {
                sInputNick: "maxcom".to_owned(),
                sCanonicalNick: "maxcom".to_owned(),
                bBlocked: false,
            },
            StMarkupUser {
                sInputNick: "isden".to_owned(),
                sCanonicalNick: "isden".to_owned(),
                bBlocked: true,
            },
        ]);
        let sRendered = render(
            "[user]maxcom[/user][USER]isden[/USER][user]hizel[/user]",
            false,
            false,
            Some("http://127.0.0.1:8080/"),
            Some(&stUsers),
            EnCutMode::Comment,
        );
        assert_eq!(
            sRendered,
            concat!(
                "<p> <span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\">",
                "<a style=\"text-decoration: none\" href=\"http://127.0.0.1:8080/people/maxcom/profile\">maxcom</a></span>",
                " <span style=\"white-space: nowrap\"><img src=\"/img/tuxlor.png\"><s>",
                "<a style=\"text-decoration: none\" href=\"http://127.0.0.1:8080/people/isden/profile\">isden</a></s></span>",
                " <s>hizel</s></p>"
            )
        );
        assert_eq!(
            render(
                "[user]maxcom[/user]",
                false,
                false,
                Some("http://127.0.0.1:8080/"),
                None,
                EnCutMode::Comment,
            ),
            "<p>maxcom</p>"
        );
    }
}
