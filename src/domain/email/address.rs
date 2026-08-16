/// The subset of Jakarta Mail's strict `InternetAddress` result that the
/// registration and SMTP paths consume.  Jakarta accepts an optional display
/// phrase, but `getAddress` returns only the addr-spec; keep that distinction
/// so the database and activation token use the same value as the JVM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StStrictInternetAddress {
    pub sAddress: String,
    pub sDomain: String,
}

fn optAddressInsidePhrase(sInput: &str) -> Option<&str> {
    let sTrimmed = sInput.trim();
    match (sTrimmed.find('<'), sTrimmed.rfind('>')) {
        (None, None) => Some(sTrimmed),
        (Some(iOpen), Some(iClose))
            if iOpen < iClose
                && sTrimmed[iClose + 1..].trim().is_empty()
                && !sTrimmed[..iOpen].contains(['<', '>'])
                && !sTrimmed[iOpen + 1..iClose].contains(['<', '>'])
                && bSingleAddressPhrase(&sTrimmed[..iOpen]) =>
        {
            Some(sTrimmed[iOpen + 1..iClose].trim())
        }
        _ => None,
    }
}

/// Jakarta treats an unquoted comma as an address-list separator and an
/// unquoted colon/semicolon as group syntax.  Its single-address constructor
/// rejects those forms, while allowing the same characters inside a quoted
/// display phrase or comment.
fn bSingleAddressPhrase(sPhrase: &str) -> bool {
    let mut bQuoted = false;
    let mut bEscaped = false;
    let mut iCommentDepth = 0u32;
    for cCharacter in sPhrase.chars() {
        if bEscaped {
            bEscaped = false;
            continue;
        }
        if cCharacter == '\\' && (bQuoted || iCommentDepth > 0) {
            bEscaped = true;
        } else if cCharacter == '"' && iCommentDepth == 0 {
            bQuoted = !bQuoted;
        } else if !bQuoted && cCharacter == '(' {
            iCommentDepth = iCommentDepth.saturating_add(1);
        } else if !bQuoted && cCharacter == ')' {
            let Some(iDepth) = iCommentDepth.checked_sub(1) else {
                return false;
            };
            iCommentDepth = iDepth;
        } else if !bQuoted && iCommentDepth == 0 && matches!(cCharacter, ',' | ':' | ';') {
            return false;
        }
    }
    !bQuoted && !bEscaped && iCommentDepth == 0
}

fn optUnquotedAt(sAddress: &str) -> Option<usize> {
    let mut bQuoted = false;
    let mut bEscaped = false;
    let mut optAt = None;
    for (iIndex, cCharacter) in sAddress.char_indices() {
        if bEscaped {
            bEscaped = false;
            continue;
        }
        if bQuoted && cCharacter == '\\' {
            bEscaped = true;
        } else if cCharacter == '"' {
            bQuoted = !bQuoted;
        } else if !bQuoted && cCharacter == '@' {
            if optAt.is_some() {
                return None;
            }
            optAt = Some(iIndex);
        }
    }
    (!bQuoted && !bEscaped).then_some(optAt).flatten()
}

fn bValidLocalPart(sLocal: &str) -> bool {
    if sLocal.is_empty() {
        return false;
    }
    if sLocal.starts_with('"') || sLocal.ends_with('"') {
        if !(sLocal.starts_with('"') && sLocal.ends_with('"') && sLocal.len() >= 2) {
            return false;
        }
        let sInner = &sLocal[1..sLocal.len() - 1];
        let mut bEscaped = false;
        for cCharacter in sInner.chars() {
            if bEscaped {
                if matches!(cCharacter, '\r' | '\n' | '\0') {
                    return false;
                }
                bEscaped = false;
            } else if cCharacter == '\\' {
                bEscaped = true;
            } else if cCharacter == '"' || cCharacter.is_control() {
                return false;
            }
        }
        return !bEscaped;
    }

    if sLocal.starts_with('.') || sLocal.ends_with('.') || sLocal.contains("..") {
        return false;
    }
    sLocal.chars().all(|cCharacter| {
        !cCharacter.is_control()
            && !cCharacter.is_whitespace()
            && !matches!(
                cCharacter,
                '(' | ')' | '<' | '>' | ',' | ';' | ':' | '\\' | '"' | '[' | ']' | '@'
            )
    })
}

fn bValidDomain(sDomain: &str) -> bool {
    if sDomain.starts_with('[') && sDomain.ends_with(']') && sDomain.len() > 2 {
        return sDomain[1..sDomain.len() - 1]
            .chars()
            .all(|cCharacter| !cCharacter.is_control() && !cCharacter.is_whitespace());
    }
    if sDomain.is_empty()
        || sDomain.len() > 253
        || sDomain.starts_with('.')
        || sDomain.ends_with('.')
    {
        return false;
    }
    sDomain.split('.').all(|sLabel| {
        !sLabel.is_empty()
            && sLabel.len() <= 63
            && sLabel
                .chars()
                .all(|cCharacter| cCharacter.is_alphanumeric() || cCharacter == '-')
    })
}

/// Parse exactly one strict Internet mailbox.  This rejects address lists and
/// the illegal dot/special forms rejected by `new InternetAddress(value,true)`
/// while retaining Jakarta's optional `Display Name <addr-spec>` form.
pub fn optParseStrictInternetAddress(sInput: &str) -> Option<StStrictInternetAddress> {
    if sInput.is_empty()
        || sInput
            .chars()
            .any(|cCharacter| cCharacter.is_control() && !matches!(cCharacter, '\t'))
    {
        return None;
    }
    let sAddress = optAddressInsidePhrase(sInput)?;
    let iAt = optUnquotedAt(sAddress)?;
    let sLocal = &sAddress[..iAt];
    let sDomain = &sAddress[iAt + 1..];
    if !bValidLocalPart(sLocal) || !bValidDomain(sDomain) {
        return None;
    }
    Some(StStrictInternetAddress {
        sAddress: sAddress.to_owned(),
        sDomain: sDomain.to_ascii_lowercase(),
    })
}

/// Jakarta callers that persist or compare a mailbox use
/// `InternetAddress.getAddress.toLowerCase`, never the optional display phrase.
pub fn optCanonicalInternetAddress(sInput: &str) -> Option<String> {
    optParseStrictInternetAddress(sInput).map(|stAddress| stAddress.sAddress.to_lowercase())
}

/// Guava `InternetDomainName.topPrivateDomain`: return the registrable domain
/// according to the public suffix list, and reject a bare public suffix.
pub fn optTopPrivateDomain(sDomain: &str) -> Option<String> {
    psl::domain(sDomain.as_bytes())
        .and_then(|stDomain| std::str::from_utf8(stDomain.as_bytes()).ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{optCanonicalInternetAddress, optParseStrictInternetAddress, optTopPrivateDomain};

    #[test]
    fn rejects_the_same_illegal_local_forms_as_jakarta_strict_mode() {
        for sAddress in [
            "a..b@example.com",
            ".a@example.com",
            "a.@example.com",
            "a;b@example.com",
            "a[b]@example.com",
            "one@two@example.com",
            "Display, Name <user@example.com>",
            "group:user@example.com;",
            "user@example.com\r\nBcc:evil@example.com",
        ] {
            assert!(
                optParseStrictInternetAddress(sAddress).is_none(),
                "{sAddress}"
            );
        }
    }

    #[test]
    fn keeps_jakarta_single_mailbox_and_get_address_behavior() {
        let stPlain = optParseStrictInternetAddress("user+tag@example.org").unwrap();
        assert_eq!(stPlain.sAddress, "user+tag@example.org");
        assert_eq!(stPlain.sDomain, "example.org");

        let stNamed = optParseStrictInternetAddress("Example User <User@Example.ORG>").unwrap();
        assert_eq!(stNamed.sAddress, "User@Example.ORG");
        assert_eq!(stNamed.sDomain, "example.org");

        let stQuotedName =
            optParseStrictInternetAddress("\"Example, User\" <user@example.org>").unwrap();
        assert_eq!(stQuotedName.sAddress, "user@example.org");
        assert!(optParseStrictInternetAddress("\"a b\"@example.org").is_some());
        assert!(optParseStrictInternetAddress("user@[127.0.0.1]").is_some());
        assert!(optParseStrictInternetAddress("user@-mail.example.org").is_some());
        assert!(optParseStrictInternetAddress("user@пример.рф").is_some());

        assert_eq!(
            optCanonicalInternetAddress("Example User <User@Example.ORG>").as_deref(),
            Some("user@example.org")
        );
    }

    #[test]
    fn public_suffix_lookup_matches_guava_top_private_domain() {
        assert_eq!(
            optTopPrivateDomain("mail.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            optTopPrivateDomain("www.linux.org.ru").as_deref(),
            Some("linux.org.ru")
        );
        assert!(optTopPrivateDomain("co.uk").is_none());
    }
}
