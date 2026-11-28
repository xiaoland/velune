//! Pi's RPC extension dialogs wait for an explicit response with their native id.
use crate::conversation::{
    InteractionKind, InteractionOption, InteractionQuestion, RuntimeInteraction,
    RuntimeInteractionReply,
};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Dialogs {
    pending: BTreeMap<String, Dialog>,
}
struct Dialog {
    kind: String,
    interaction: RuntimeInteraction,
    deadline: Option<Instant>,
}

pub(crate) fn is_dialog(event: &Value) -> bool {
    event["type"] == "extension_ui_request"
        && matches!(
            event["method"].as_str(),
            Some("select" | "confirm" | "input" | "editor")
        )
}
impl Dialogs {
    pub(crate) fn observe(&mut self, event: &Value) {
        if matches!(
            event["type"].as_str(),
            Some("agent_settled" | "velune_error")
        ) {
            self.pending.clear();
            return;
        }
        if !is_dialog(event) {
            return;
        }
        let Some(id) = event["id"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        let method = event["method"].as_str().unwrap();
        let title = event["title"].as_str().unwrap_or("Pi 请求输入").to_owned();
        let options = match method {
            "confirm" => [("yes", "是"), ("no", "否")]
                .into_iter()
                .map(|(id, label)| InteractionOption {
                    id: id.into(),
                    label: label.into(),
                    approval_kind: None,
                })
                .collect(),
            "select" => event["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .enumerate()
                .map(|(index, label)| InteractionOption {
                    id: index.to_string(),
                    label: label.into(),
                    approval_kind: None,
                })
                .collect(),
            _ => vec![],
        };
        let kind = InteractionKind::UserInput {
            questions: vec![InteractionQuestion {
                id: id.into(),
                text: [
                    Some(title.as_str()),
                    event["message"].as_str(),
                    event["placeholder"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n"),
                options,
                secret: false,
                default_text: event["prefill"].as_str().map(str::to_owned),
                multiline: method == "editor",
                allow_other: matches!(method, "input" | "editor"),
                multiple: false,
            }],
        };
        self.pending.insert(
            id.into(),
            Dialog {
                kind: method.into(),
                interaction: RuntimeInteraction {
                    id: id.into(),
                    kind,
                },
                deadline: event["timeout"]
                    .as_u64()
                    .filter(|ms| *ms > 0)
                    .and_then(|ms| Instant::now().checked_add(Duration::from_millis(ms))),
            },
        );
    }
    pub(crate) fn expire(&mut self) {
        let now = Instant::now();
        self.pending
            .retain(|_, dialog| dialog.deadline.is_none_or(|deadline| deadline > now));
    }
    pub(crate) fn interactions(&self) -> Vec<RuntimeInteraction> {
        self.pending
            .values()
            .map(|d| d.interaction.clone())
            .collect()
    }
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }
    pub(crate) fn remove(&mut self, id: &str) {
        self.pending.remove(id);
    }
    pub(crate) fn response(&mut self, id: &str, reply: RuntimeInteractionReply) -> Result<Value> {
        self.expire();
        let dialog = self
            .pending
            .get(id)
            .ok_or_else(|| Error::Sdk("Pi 请求已结束或过期".into()))?;
        let mut response = json!({"type":"extension_ui_response","id":id});
        match reply {
            RuntimeInteractionReply::Cancel => response["cancelled"] = json!(true),
            RuntimeInteractionReply::Decision { .. } => {
                return Err(Error::Sdk("Pi 对话框需要问答回复".into()));
            }
            RuntimeInteractionReply::Answers { answers } => {
                if answers.len() != 1
                    || answers[0].question_id != id
                    || answers[0].values.len() != 1
                {
                    return Err(Error::Sdk("Pi 请求答案无效".into()));
                }
                let value = &answers[0].values[0];
                if matches!(dialog.kind.as_str(), "select" | "confirm") {
                    let InteractionKind::UserInput { questions } = &dialog.interaction.kind else {
                        unreachable!()
                    };
                    let option = questions[0]
                        .options
                        .iter()
                        .find(|option| option.id == *value)
                        .ok_or_else(|| Error::Sdk("Pi 选项无效".into()))?;
                    if dialog.kind == "confirm" {
                        response["confirmed"] = json!(option.id == "yes");
                    } else {
                        response["value"] = json!(option.label);
                    }
                } else {
                    response["value"] = json!(value);
                }
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::InteractionAnswer;
    #[test]
    fn dialogs_preserve_native_values_prefill_and_explicit_cancellation() {
        let answer = |id: &str, value: &str| RuntimeInteractionReply::Answers {
            answers: vec![InteractionAnswer {
                question_id: id.into(),
                values: vec![value.into()],
            }],
        };
        let mut dialogs = Dialogs::default();
        dialogs.observe(&json!({"type":"extension_ui_request","id":"s","method":"select","title":"choose","options":["A","B"]}));
        assert_eq!(
            dialogs.response("s", answer("s", "1")).unwrap()["value"],
            "B"
        );
        assert!(dialogs.response("s", answer("s", "B")).is_err());
        dialogs.observe(
            &json!({"type":"extension_ui_request","id":"c","method":"confirm","title":"confirm"}),
        );
        assert_eq!(
            dialogs.response("c", answer("c", "no")).unwrap()["confirmed"],
            false
        );
        dialogs.observe(&json!({"type":"extension_ui_request","id":"e","method":"editor","title":"edit","prefill":"a\nb"}));
        let InteractionKind::UserInput { questions } = &dialogs.pending["e"].interaction.kind
        else {
            panic!("text question");
        };
        assert!(questions[0].multiline);
        assert_eq!(questions[0].default_text.as_deref(), Some("a\nb"));
        let reply = RuntimeInteractionReply::Answers {
            answers: vec![InteractionAnswer {
                question_id: "e".into(),
                values: vec!["edited\ntext".into()],
            }],
        };
        assert_eq!(
            dialogs.response("e", reply).unwrap()["value"],
            "edited\ntext"
        );
        assert_eq!(
            dialogs
                .response("e", RuntimeInteractionReply::Cancel)
                .unwrap()["cancelled"],
            true
        );
        dialogs.observe(&json!({"type":"agent_settled"}));
        assert!(dialogs.interactions().is_empty());
    }
    #[test]
    fn expired_dialogs_cannot_be_answered_late() {
        let mut dialogs = Dialogs::default();
        dialogs.observe(&json!({"type":"extension_ui_request","id":"i","method":"input"}));
        dialogs.pending.get_mut("i").unwrap().deadline = Some(Instant::now());
        assert!(
            dialogs
                .response("i", RuntimeInteractionReply::Cancel)
                .is_err()
        );
        assert!(dialogs.interactions().is_empty());
    }
    #[test]
    fn unrelated_output_does_not_answer_dialogs_and_native_end_or_disconnect_expires_them() {
        let request = json!({"type":"extension_ui_request","id":"input","method":"input","title":"Native input"});
        let mut dialogs = Dialogs::default();
        dialogs.observe(&request);
        for event in [
            json!({"type":"message_end"}),
            json!({"type":"agent_end"}),
            json!({"type":"extension_ui_request","id":"other","method":"notify"}),
        ] {
            dialogs.observe(&event);
            assert_eq!(dialogs.interactions().len(), 1);
        }
        // Producing a reply does not consume the dialog before its native write
        // succeeds. PiSession owns removal after the native reply is Written.
        let response = dialogs
            .response("input", RuntimeInteractionReply::Cancel)
            .unwrap();
        assert_eq!(response["cancelled"], true);
        assert_eq!(dialogs.interactions().len(), 1);
        dialogs.remove("input");
        assert!(dialogs.interactions().is_empty());
        assert!(
            dialogs
                .response("input", RuntimeInteractionReply::Cancel)
                .is_err()
        );
        for event in [
            json!({"type":"agent_settled"}),
            json!({"type":"velune_error","error":"synthetic disconnected"}),
        ] {
            dialogs.observe(&request);
            dialogs.observe(&event);
            assert!(dialogs.interactions().is_empty());
            assert!(
                dialogs
                    .response("input", RuntimeInteractionReply::Cancel)
                    .is_err()
            );
        }
    }
}
