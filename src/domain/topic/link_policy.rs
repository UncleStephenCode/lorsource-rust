/// Minimum author score at which Java's `TopicPermissionService` permits
/// search engines to follow links in user-authored content.
pub const LINK_FOLLOW_MIN_SCORE: i32 = 100;

/// Author state used by the content-link policy. Activation is deliberately
/// absent: `TopicPermissionService.followAuthorLinks` does not inspect it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StAuthorLinkState {
    pub iScore: i32,
    pub bBlocked: bool,
    pub bAnonymous: bool,
    pub bFrozen: bool,
}

impl StAuthorLinkState {
    pub const fn bFollowAuthorLinks(self) -> bool {
        !self.bBlocked && !self.bAnonymous && !self.bFrozen && self.iScore >= LINK_FOLLOW_MIN_SCORE
    }

    /// `TopicPermissionService.followInTopic`: moderation/commit approval is
    /// sufficient by itself, even for an anonymous, blocked or frozen author.
    pub const fn bFollowInTopic(self, bCommitted: bool) -> bool {
        bCommitted || self.bFollowAuthorLinks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stAuthor(iScore: i32) -> StAuthorLinkState {
        StAuthorLinkState {
            iScore,
            bBlocked: false,
            bAnonymous: false,
            bFrozen: false,
        }
    }

    #[test]
    fn follows_only_registered_unrestricted_authors_at_java_threshold() {
        assert!(!stAuthor(99).bFollowAuthorLinks());
        assert!(stAuthor(100).bFollowAuthorLinks());

        assert!(
            !StAuthorLinkState {
                bBlocked: true,
                ..stAuthor(3000)
            }
            .bFollowAuthorLinks()
        );
        assert!(
            !StAuthorLinkState {
                bAnonymous: true,
                ..stAuthor(3000)
            }
            .bFollowAuthorLinks()
        );
        assert!(
            !StAuthorLinkState {
                bFrozen: true,
                ..stAuthor(3000)
            }
            .bFollowAuthorLinks()
        );
    }

    #[test]
    fn committed_topic_overrides_every_author_restriction() {
        let stRestricted = StAuthorLinkState {
            iScore: -100,
            bBlocked: true,
            bAnonymous: true,
            bFrozen: true,
        };
        assert!(!stRestricted.bFollowInTopic(false));
        assert!(stRestricted.bFollowInTopic(true));
    }
}
