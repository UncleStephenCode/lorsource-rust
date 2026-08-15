use askama::Template;

#[derive(Debug)]
struct StMoveGroupView<'a> {
    id: i32,
    label: &'a str,
    selected: bool,
}

#[derive(Template)]
#[template(path = "uncommit_topic.html")]
struct StUncommitTemplate<'a> {
    csrf_token: &'a str,
    topic_id: i32,
    topic_card_html: &'a str,
}

#[derive(Template)]
#[template(path = "move_topic.html")]
struct StMoveTemplate<'a> {
    csrf_token: &'a str,
    topic_id: i32,
    groups: &'a [StMoveGroupView<'a>],
    author_nick: &'a str,
    author_score: i32,
    author_blocked: bool,
}

#[derive(Template)]
#[template(path = "topic_moderation_forbidden.html")]
struct StForbiddenTemplate<'a> {
    message: &'a str,
}

#[test]
fn uncommit_page_keeps_the_original_form_and_embeds_a_menu_free_topic_card() {
    let sHtml = StUncommitTemplate {
        csrf_token: "csrf-token",
        topic_id: 42,
        topic_card_html: "<article class=\"msg\" data-show-menu=\"false\">topic</article>",
    }
    .render()
    .unwrap();

    assert!(sHtml.contains("<title>Возврат в неподтверждённые</title>"));
    assert!(sHtml.contains("<h1>Возврат в неподтверждённые</h1>"));
    assert!(
        sHtml.contains(
            "Вы можете отменить подтверждение и вернуть топик в список неподтверждённых."
        )
    );
    assert!(sHtml.contains("<div class=\"messages\">"));
    assert!(sHtml.contains("<article class=\"msg\" data-show-menu=\"false\">topic</article>"));
    assert!(sHtml.contains("<form method=\"post\" action=\"uncommit.jsp\">"));
    assert!(sHtml.contains("name=\"csrf\" value=\"csrf-token\""));
    assert!(sHtml.contains("name=\"msgid\" value=\"42\""));
    assert!(sHtml.contains(
        "type=\"submit\" name=\"undel\" class=\"btn btn-primary\">Отменить подтверждение"
    ));
}

#[test]
fn move_page_keeps_absolute_action_labels_selection_and_plain_author_text() {
    let vecGroups = [
        StMoveGroupView {
            id: 10,
            label: "Форум: Linux",
            selected: true,
        },
        StMoveGroupView {
            id: 11,
            label: "Статьи: Rust & безопасность",
            selected: false,
        },
    ];
    let sHtml = StMoveTemplate {
        csrf_token: "csrf-token",
        topic_id: 42,
        groups: &vecGroups,
        author_nick: "blocked-user",
        author_score: 300,
        author_blocked: true,
    }
    .render()
    .unwrap();

    assert!(sHtml.contains("<title>Перенос топика</title>"));
    assert!(sHtml.contains("<h1>Перенос топика</h1>"));
    assert!(sHtml.contains("<form method=\"post\" action=\"/mt.jsp\" style=\"margin-top: 1em\">"));
    assert!(sHtml.contains("<select name=\"moveto\">"));
    assert!(sHtml.contains("<option value=\"10\" selected=\"selected\">Форум: Linux</option>"));
    assert!(sHtml.contains("Статьи: Rust &#38; безопасность"));
    assert!(sHtml.contains("<button type=\"submit\" class=\"btn btn-primary\">Переместить"));
    assert!(sHtml.contains("Сообщение написано <s>blocked-user</s>, score=300"));
    assert!(!sHtml.contains("/people/blocked-user/profile"));
}

#[test]
fn moderation_denials_can_preserve_the_source_exception_message() {
    let sHtml = StForbiddenTemplate {
        message: "В данной группе нельзя помечать темы как решенные",
    }
    .render()
    .unwrap();
    assert!(sHtml.contains("<h1>403 Forbidden</h1>"));
    assert!(sHtml.contains("<p>В данной группе нельзя помечать темы как решенные.</p>"));
}

#[test]
fn resolve_has_no_form_template_because_the_original_is_a_direct_redirect_action() {
    let sApplication = include_str!("../src/application/topic/moderation.rs");
    assert!(sApplication.contains("pub async fn stResolve"));
    assert!(sApplication.contains("sRedirectUrl: stTopic.sForceLastModUrl()"));
    assert!(!sApplication.contains("StPreparedResolve"));
}

#[test]
fn canonical_and_uncommit_pages_share_the_full_topic_card_renderer() {
    let sCanonical = include_str!("../templates/topic.html");
    let sUncommit = include_str!("../templates/uncommit_topic.html");
    let sCard = include_str!("../templates/topic_card.html");
    let sRoutes = include_str!("../src/routes/topic_moderation.rs");
    let sTopics = include_str!("../src/routes/topics.rs");

    assert!(sCanonical.contains("{{ topic_card_html|safe }}"));
    assert!(sUncommit.contains("{{ topic_card_html|safe }}"));
    assert_eq!(sCanonical.matches("<article class=\"msg\"").count(), 0);
    assert_eq!(sUncommit.matches("<article class=\"msg\"").count(), 0);
    assert!(sCard.contains("<article class=\"msg\" id=\"topic-{{ card.topic.id }}\""));
    assert!(sCard.contains("{% if card.show_menu %}"));
    assert!(sCard.contains("card.deleted_header_html|safe"));
    assert!(sCard.contains("card.committer_html|safe"));
    assert!(sCard.contains("card.moderator_user_agent_html|safe"));
    assert!(sCard.contains("card.edit_summary_html|safe"));
    assert!(sCard.contains("card.warnings_html|safe"));
    assert!(sCard.contains("card.memories_buttons_html|safe"));
    assert!(sRoutes.contains("sPrepareTopicCardHtml("));
    assert!(sRoutes.contains("false,"));
    assert!(sTopics.contains("include_canonical_extras: true"));
    assert!(sTopics.contains("include_canonical_extras: false"));
}

#[test]
fn edit_preview_uses_the_common_topic_card() {
    let sEditPreview = include_str!("../templates/edit_topic.html");
    assert!(sEditPreview.contains("{{ topic_card_html|safe }}"));
}
