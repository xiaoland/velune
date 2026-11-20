//! Async questions are assistant output, not server requests awaiting an RPC reply.
use crate::conversation::{
    AsyncQuestionDelivery, InteractionKind, InteractionOption, InteractionQuestion, Message,
    MessageBlock, MessageRole, RuntimeInteraction,
};
use serde_json::Value;

pub(super) fn project(item: &Value) -> Option<RuntimeInteraction> {
    if item["type"] != "agentMessage" {
        return None;
    }
    let id = item["id"].as_str()?;
    let questions = item["questions"]
        .as_array()?
        .iter()
        .enumerate()
        .map(|(index, question)| {
            Some(InteractionQuestion {
                id: index.to_string(),
                text: question["title"].as_str()?.to_owned(),
                options: question["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        let label = option.as_str()?;
                        Some(InteractionOption {
                            id: label.into(),
                            label: label.into(),
                            approval_kind: None,
                        })
                    })
                    .collect(),
                secret: false,
                default_text: None,
                multiline: false,
                allow_other: true,
                multiple: false,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if questions.is_empty() || questions.iter().any(|q| q.text.trim().is_empty()) {
        return None;
    }
    Some(RuntimeInteraction {
        id: format!("codex-async:{id}"),
        kind: InteractionKind::AsyncUserInput {
            questions,
            delivery: crate::conversation::AsyncQuestionDelivery::ConversationInput,
        },
    })
}

/// Only an accepted native user item is an answer. A pending/rejected steer,
/// a later assistant reply, or merely ending a turn does not settle a question.
fn input(item: &Value) -> Option<String> {
    let content = match item["type"].as_str()? {
        "userMessage" => item["content"].as_array()?,
        "steeringUserMessage"
            if item["status"] == "accepted" || item["steerAcknowledged"] == true =>
        {
            item["input"].as_array()?
        }
        _ => return None,
    };
    Some(
        content
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Codex desktop's uHDr formatter uses a language-independent envelope. The
/// question id is either the original item id (old single-question UI) or the
/// JSON tuple ["request_user_input_async", item id, zero-based question index].
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    question_item_id: String,
    question: String,
    answer: String,
}
fn structured_answers(text: &str) -> Option<Vec<Answer>> {
    let text = text.rsplit("\n## My request:\n").next()?.trim();
    let text = if text.starts_with("[from: ") {
        let (sender, body) = text.split_once('\n')?;
        if !sender.ends_with(']') {
            return None;
        }
        body.trim().to_owned()
    } else if text.starts_with("<codex_multi_user_message>")
        && text.ends_with("</codex_multi_user_message>")
    {
        let (_, body) = text.split_once("<body>")?;
        let (body, _) = body.split_once("</body>")?;
        body.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&")
    } else {
        text.to_owned()
    };
    let raw = text
        .trim()
        .strip_prefix("<send_user_message_question_reply>")?
        .strip_suffix("</send_user_message_question_reply>")?
        .trim();
    let value: Value = serde_json::from_str(raw).ok()?;
    let answers = if value.is_array() {
        serde_json::from_value(value).ok()?
    } else {
        vec![serde_json::from_value(value).ok()?]
    };
    (!answers.is_empty()).then_some(answers)
}
fn names_question(answer: &Answer, source: &str, question: &InteractionQuestion) -> bool {
    if answer.answer.trim().is_empty() || answer.question != question.text {
        return false;
    }
    if answer.question_item_id == source {
        return true;
    }
    let Ok(Value::Array(id)) = serde_json::from_str::<Value>(&answer.question_item_id) else {
        return false;
    };
    id.len() == 3
        && id[0] == "request_user_input_async"
        && id[1] == source
        && id[2].as_u64().map(|index| index.to_string()).as_deref() == Some(question.id.as_str())
}
/// Recognize the exact older Chinese desktop/Velune answer format. This is a
/// compatibility path for observed historical records, not a guess that any
/// user message following a question answers it. Identical titles are ambiguous.
fn legacy_answers(text: &str, questions: &[(&str, &InteractionQuestion)]) -> Vec<(String, String)> {
    let matches_at = |offset| {
        questions
            .iter()
            .filter(|(_, q)| {
                text[offset..]
                    .strip_prefix(&q.text)
                    .and_then(|rest| rest.strip_prefix("\n回答："))
                    .is_some_and(|answer| !answer.trim().is_empty())
            })
            .collect::<Vec<_>>()
    };
    if matches_at(0).len() != 1 {
        return Vec::new();
    }
    std::iter::once(0)
        .chain(text.match_indices("\n\n").map(|(offset, _)| offset + 2))
        .filter_map(|offset| match matches_at(offset).as_slice() {
            [(source, question)] => Some(((*source).to_owned(), question.id.clone())),
            _ => None,
        })
        .collect()
}
fn resolve_text(
    pending: &mut Vec<RuntimeInteraction>,
    text: &str,
    eligible: impl Fn(&str) -> bool,
) -> bool {
    let candidates = pending
        .iter()
        .filter_map(|interaction| {
            let source = interaction.id.strip_prefix("codex-async:")?;
            if !eligible(source) {
                return None;
            }
            let InteractionKind::AsyncUserInput {
                questions,
                delivery: AsyncQuestionDelivery::ConversationInput,
            } = &interaction.kind
            else {
                return None;
            };
            Some(questions.iter().map(move |question| (source, question)))
        })
        .flatten()
        .collect::<Vec<_>>();
    let resolved = if let Some(answers) = structured_answers(text) {
        answers
            .iter()
            .flat_map(|answer| {
                let matches = candidates
                    .iter()
                    .filter(|(source, question)| names_question(answer, source, question))
                    .collect::<Vec<_>>();
                match matches.as_slice() {
                    [(source, question)] => vec![((*source).to_owned(), question.id.clone())],
                    _ => vec![],
                }
            })
            .collect::<Vec<_>>()
    } else {
        legacy_answers(text, &candidates)
    };
    let mut changed = false;
    pending.retain_mut(|interaction| {
        let Some(source) = interaction.id.strip_prefix("codex-async:") else {
            return true;
        };
        let InteractionKind::AsyncUserInput {
            questions,
            delivery: AsyncQuestionDelivery::ConversationInput,
        } = &mut interaction.kind
        else {
            return true;
        };
        let before = questions.len();
        questions.retain(|question| {
            !resolved
                .iter()
                .any(|(id, q)| id == source && q == &question.id)
        });
        changed |= questions.len() != before;
        !questions.is_empty()
    });
    changed
}
pub(super) fn resolve_item(pending: &mut Vec<RuntimeInteraction>, item: &Value) -> bool {
    input(item).is_some_and(|text| resolve_text(pending, &text, |_| true))
}
/// Preserve native ordering: a response before its question cannot answer it.
pub(super) fn pending(items: &[Value]) -> Vec<RuntimeInteraction> {
    let mut pending = Vec::new();
    for entry in items {
        let item = &entry["item"];
        if let Some(question) = project(item) {
            pending.push(question);
        }
        resolve_item(&mut pending, item);
    }
    pending
}
/// Repeated started/completed items must not resurrect an answered card. The
/// source message's position bounds which accepted user messages can resolve it.
pub(super) fn reconcile(pending: &mut Vec<RuntimeInteraction>, messages: &[Message]) -> bool {
    let positions = messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut changed = false;
    for (index, message) in messages.iter().enumerate() {
        if pending.is_empty() {
            break;
        }
        if message.role != MessageRole::User {
            continue;
        }
        let text = message
            .blocks
            .iter()
            .filter_map(|block| {
                if let MessageBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        changed |= resolve_text(pending, &text, |source| {
            positions
                .get(source)
                .is_some_and(|position| *position < index)
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn questions_survive_as_nonblocking_interactions_without_rpc_ids() {
        let item = json!({"type":"agentMessage","id":"ask","delivery":"async","text":"question", "questions":[{"title":"Choose","options":["A","B"]},{"title":"Details","options":null}]});
        let interaction = project(&item).unwrap();
        assert_eq!(interaction.id, "codex-async:ask");
        assert!(!interaction.blocks_turn());
        let InteractionKind::AsyncUserInput { questions, .. } = interaction.kind else {
            panic!()
        };
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].options[1].label, "B");
        assert!(questions[1].options.is_empty());
        assert!(
            project(&json!({"type":"agentMessage","id":"text","text":"ordinary reply"})).is_none()
        );
    }
    fn ask(id: &str, titles: &[&str]) -> Value {
        json!({"type":"agentMessage","id":id,"text":"synthetic question","delivery":"async",
            "questions": titles.iter().map(|title| json!({"title":title})).collect::<Vec<_>>()})
    }
    fn user(text: &str) -> Value {
        json!({"type":"userMessage","id":"answer","content":[{"type":"text","text":text}]})
    }
    fn reply(id: &str, index: usize, title: &str) -> Value {
        json!({"questionItemId":json!(["request_user_input_async",id,index]).to_string(),"question":title,"answer":"synthetic choice"})
    }
    fn envelope(answers: Value) -> String {
        format!(
            "<send_user_message_question_reply>\n{answers}\n</send_user_message_question_reply>"
        )
    }
    fn history(items: Vec<Value>) -> Vec<Value> {
        items.into_iter().map(|item| json!({"item":item})).collect()
    }
    #[test]
    fn native_typed_answers_remove_only_exact_questions_in_history_and_live() {
        let first = ask("ask", &["First?", "第二题？"]);
        let unrelated = ask("other", &["First?"]);
        let text = envelope(json!([reply("ask", 1, "第二题？")]));
        let mut active = vec![project(&first).unwrap(), project(&unrelated).unwrap()];
        assert!(resolve_item(&mut active, &user(&text)));
        let InteractionKind::AsyncUserInput { questions, .. } = &active[0].kind else {
            panic!()
        };
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "0");
        assert_eq!(active[1].id, "codex-async:other");
        let rest = user(&envelope(reply("ask", 0, "First?")));
        assert!(resolve_item(&mut active, &rest));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "codex-async:other");
        let cold = pending(&history(vec![first, unrelated, user(&text), rest]));
        assert_eq!(cold, active);
    }
    #[test]
    fn formatter_single_array_context_and_multi_user_envelopes_are_language_independent() {
        let question = ask("ask", &["任意语言 / café?", "Another?"]);
        let answers = json!([
            reply("ask", 0, "任意语言 / café?"),
            reply("ask", 1, "Another?")
        ]);
        let raw = envelope(answers);
        let xml = raw
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        for text in [
            raw.clone(),
            format!("context\n## My request:\n{raw}"),
            format!("[from: synthetic-user]\n{raw}"),
            format!(
                "<codex_multi_user_message>\n<account_user_id>synthetic</account_user_id>\n<name>User</name>\n<email>local</email>\n<profile_picture_url></profile_picture_url>\n<body>{xml}</body>\n</codex_multi_user_message>"
            ),
        ] {
            assert!(pending(&history(vec![question.clone(), user(&text)])).is_empty());
        }
        let legacy_id = json!({"questionItemId":"ask","question":"Another?","answer":"yes"});
        let resolved = pending(&history(vec![question, user(&envelope(legacy_id))]));
        let InteractionKind::AsyncUserInput { questions, .. } = &resolved[0].kind else {
            panic!()
        };
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "0");
    }
    #[test]
    fn unrelated_stale_malformed_and_unaccepted_replies_do_not_dismiss() {
        let question = ask("ask", &["Choose?"]);
        let valid = envelope(reply("ask", 0, "Choose?"));
        for text in [
            "A later message".into(),
            "回答：yes".into(),
            "intro\nChoose?\n回答：yes".into(),
            envelope(reply("other", 0, "Choose?")),
            envelope(reply("ask", 1, "Choose?")),
            envelope(reply("ask", 0, "Different?")),
            format!("quoted {valid}"),
            envelope(json!([reply("ask",0,"Choose?"),{"questionItemId":"broken"}])),
        ] {
            assert_eq!(
                pending(&history(vec![question.clone(), user(&text)])).len(),
                1,
                "{text}"
            );
        }
        // A future question with the same id cannot be answered retroactively.
        assert_eq!(
            pending(&history(vec![user(&valid), question.clone()])).len(),
            1
        );
        for status in ["pending", "rejected"] {
            let steering = json!({"type":"steeringUserMessage","id":"s","status":status,"input":[{"type":"text","text":valid}]});
            assert_eq!(pending(&history(vec![question.clone(), steering])).len(), 1);
        }
        let steering = json!({"type":"steeringUserMessage","id":"s","status":"accepted","input":[{"type":"text","text":valid}]});
        assert!(pending(&history(vec![question.clone(), steering])).is_empty());
        let mut native = project(&question).unwrap();
        let InteractionKind::AsyncUserInput { delivery, .. } = &mut native.kind else {
            panic!()
        };
        *delivery = AsyncQuestionDelivery::NativeReply;
        let mut native = vec![native];
        assert!(!resolve_item(&mut native, &user(&valid)));
        assert_eq!(native.len(), 1);
    }
    #[test]
    fn legacy_exact_answer_format_supports_partial_multi_question_and_rejects_ambiguity() {
        let first = ask("ask", &["First?", "第二题？"]);
        let text = "First?\n回答：yes\n\n第二题？\n回答：多行\n\n自由回答";
        assert!(pending(&history(vec![first.clone(), user(text)])).is_empty());
        let partial = pending(&history(vec![first.clone(), user("第二题？\n回答：yes")]));
        let InteractionKind::AsyncUserInput { questions, .. } = &partial[0].kind else {
            panic!()
        };
        assert_eq!(questions[0].text, "First?");
        assert_eq!(
            pending(&history(vec![
                first,
                ask("duplicate", &["First?"]),
                user("First?\n回答：yes")
            ]))
            .len(),
            2
        );
    }
    #[test]
    fn duplicate_question_events_cannot_resurrect_resolved_cards() {
        let question = ask("ask", &["Choose?"]);
        let answer = user(&envelope(reply("ask", 0, "Choose?")));
        let messages = [
            super::super::codex::project_item(&question, "synthetic").unwrap(),
            super::super::codex::project_item(&answer, "synthetic").unwrap(),
        ];
        let mut repeated = vec![project(&question).unwrap()];
        assert!(reconcile(&mut repeated, &messages));
        assert!(repeated.is_empty());
        let mut future = vec![project(&question).unwrap()];
        assert!(!reconcile(
            &mut future,
            &[messages[1].clone(), messages[0].clone()]
        ));
        assert_eq!(future.len(), 1);
    }
}
