# Add-topic permission compatibility

The Rust `/add.jsp` GET and POST handlers enforce the current Java
`AddTopicChecker` rules for authenticated sessions:

- `max(groups.restrict_topics, sections.restrict_topics)`;
- all special postscore values and exact user-facing restriction reasons;
- current `users.frozen_until` state;
- active `b_ips` rows, including `ban_date` expiry and `allow_posting`;
- the Java restriction-chain order (frozen, postscore, IP block).

The client address used for `b_ips` is the TCP peer address, matching
`HttpServletRequest.getRemoteAddr`. Deployments that terminate connections in
a reverse proxy must preserve the same remote-address behavior they used for
the Java service; arbitrary forwarded headers are intentionally not trusted.

The same GET/POST flow now also applies Java's `TopicPublishChecker` and
`FloodProtector.AddTopic` policies:

- ordinary authenticated users may publish at least two topics per section in
  a rolling 24-hour window, rising to three, four, or five with green stars;
- moderators and active correctors are exempt;
- drafts do not count and may still be saved after the daily publishing limit
  is reached; uncommitted topics in premoderated sections do count;
- self-deleted topics continue to count, while topics deleted by another user
  do not, matching `TopicDao.countRecentTopics`;
- previews bypass flood protection, while publishes and drafts share the
  Java per-IP `AddTopic` cache;
- the interval is 30 seconds for users with score at least 100 unless slow
  mode applies, and 10 minutes otherwise;
- slow mode mirrors the score-under-35, recent-freeze, and three-day deletion
  score-loss rules; the cache is disabled only when `PUBLIC_URL` has the exact
  host `127.0.0.1`.

The variable named `dupeProtector` in `AddTopicController` is an instance of
`FloodProtector`; current Java does not compare title/body content for duplicate
topics. The Rust port therefore preserves its actual action-plus-IP/time
semantics rather than inventing content deduplication.

## Remaining deliberate gap

Java permits a non-authorized request to become a posting session through the
dedicated anonymous database user or through form `nick`/`password`, with
request-dependent CAPTCHA validation (`AuthUtil.postingUser` and
`captchaRequired`). The Rust form does not yet implement that complete
identity/CAPTCHA contract. Anonymous GET/POST `/add.jsp` therefore remains
forbidden; the port does not fabricate a user id or bypass the CAPTCHA.
