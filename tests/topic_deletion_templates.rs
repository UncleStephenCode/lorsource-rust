use askama::Template;

#[derive(Template)]
#[template(path = "delete_topic.html")]
struct StDeleteTopicTemplate<'a> {
    csrf_token: &'a str,
    topic_id: i32,
    draft: bool,
    moderator: bool,
    uncommitted: bool,
    bonus_eligible: bool,
    author_score: i32,
    delete_reasons: &'a [&'a str],
}

#[derive(Template)]
#[template(path = "undelete_topic.html")]
struct StUndeleteTopicTemplate<'a> {
    csrf_token: &'a str,
    topic_id: i32,
    topic_card_html: &'a str,
}

#[test]
fn author_delete_form_keeps_required_reason_and_the_historic_six_hour_copy() {
    let sHtml = StDeleteTopicTemplate {
        csrf_token: "csrf-token",
        topic_id: 42,
        draft: false,
        moderator: false,
        uncommitted: false,
        bonus_eligible: true,
        author_score: 100,
        delete_reasons: &[],
    }
    .render()
    .unwrap();

    assert!(sHtml.contains("<title>Удаление сообщения</title>"));
    assert!(sHtml.contains("<h1>Удаление сообщения</h1>"));
    assert!(sHtml.contains("function change(dest,source)"));
    assert!(sHtml.contains("dest.value = source.options[source.selectedIndex].value;"));
    assert!(sHtml.contains("Вы можете удалить своё сообщение в течение 6 часов"));
    assert!(sHtml.contains("<form method=POST action=\"delete.jsp\" class=\"form-horizontal\">"));
    assert!(sHtml.contains("name=reason"));
    assert!(sHtml.contains("name=msgid value=\"42\""));
    assert!(sHtml.contains("name=\"csrf\" value=\"csrf-token\""));
    assert!(!sHtml.contains("name=reason_select"));
    assert!(!sHtml.contains("name=bonus"));
}

#[test]
fn moderator_form_has_ordered_reasons_bonus_and_uncommitted_help() {
    let sHtml = StDeleteTopicTemplate {
        csrf_token: "csrf-token",
        topic_id: 43,
        draft: false,
        moderator: true,
        uncommitted: true,
        bonus_eligible: true,
        author_score: -5,
        delete_reasons: &["3.1 Дубль", "4.6 Спам & abuse"],
    }
    .render()
    .unwrap();

    let iDuplicate = sHtml.find("3.1 Дубль").unwrap();
    let iSpam = sHtml.find("4.6 Спам &#38; abuse").unwrap();
    assert!(iDuplicate < iSpam);
    assert!(sHtml.contains("name=reason_select"));
    assert!(sHtml.contains("name=bonus value=\"7\" min=\"0\" max=\"20\""));
    assert!(sHtml.contains("score автора: -5"));
    assert!(sHtml.contains("Сообщения, удалённые с пустой причиной"));
}

#[test]
fn draft_form_omits_copy_and_bonus_even_for_a_moderator() {
    let sHtml = StDeleteTopicTemplate {
        csrf_token: "csrf-token",
        topic_id: 44,
        draft: true,
        moderator: true,
        uncommitted: false,
        bonus_eligible: false,
        author_score: 100,
        delete_reasons: &[],
    }
    .render()
    .unwrap();

    assert!(!sHtml.contains("в течение 6 часов"));
    assert!(!sHtml.contains("name=bonus"));
}

#[test]
fn undelete_form_embeds_the_full_menu_free_topic_card_and_exact_fields() {
    let sTopicCard = concat!(
        "<article class=\"msg\" id=\"topic-42\">",
        "<header><h1>Full topic</h1></header>",
        "<div class=\"msg-container\"><div class=\"msg_body\">",
        "<div class=\"msg-text\"><p>Body</p></div>",
        "<footer><div class=\"sign\" style=\"margin-left: 0\">author</div></footer>",
        "</div></div></article>"
    );
    let sHtml = StUndeleteTopicTemplate {
        csrf_token: "csrf-token",
        topic_id: 42,
        topic_card_html: sTopicCard,
    }
    .render()
    .unwrap();

    assert!(sHtml.contains("<title>Восстановление сообщения</title>"));
    assert!(sHtml.contains("Вы можете восстановить удалённое сообщение."));
    assert!(sHtml.contains(sTopicCard));
    assert!(sHtml.contains("<form method=POST action=\"undelete\">"));
    assert!(sHtml.contains("name=msgid value=\"42\""));
    assert!(sHtml.contains("type=submit name=undel class=\"btn btn-primary\">Восстановить"));
}
