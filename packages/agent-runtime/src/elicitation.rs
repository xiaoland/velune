//! The scalar MCP form subset expressible by the shared question card.
use crate::conversation::*;
use crate::native::{Error, Result};
use serde_json::{Value, json};

pub(crate) fn questions(params: &Value) -> Result<Vec<InteractionQuestion>> {
    if params["mode"].as_str().unwrap_or("form") != "form"
        || params["requestedSchema"]["type"] != "object"
    {
        return Err(Error::new(
            "此 MCP 提问需要尚未支持的专用界面（URL、验证或非对象表单）",
        ));
    }
    let schema = &params["requestedSchema"];
    let fields = schema["properties"]
        .as_object()
        .ok_or_else(|| Error::new("MCP 表单缺少字段"))?;
    let mut questions = Vec::new();
    for (id, field) in fields {
        if !matches!(
            field["type"].as_str(),
            Some("string" | "number" | "integer" | "boolean")
        ) {
            return Err(Error::new("此 MCP 表单包含尚不支持的字段类型"));
        }
        let mut options = options(field);
        let allow_other = options.is_empty();
        if !required(schema, id) {
            options.push(InteractionOption {
                id: omission_id(id),
                label: "不填写（可选）".into(),
                approval_kind: None,
            });
        }
        let title = field["title"].as_str().unwrap_or(id);
        let text = [
            params["message"].as_str(),
            Some(title),
            field["description"].as_str(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
        questions.push(InteractionQuestion {
            id: id.clone(),
            text,
            options,
            secret: false,
            default_text: None,
            multiline: false,
            allow_other,
            multiple: false,
        });
    }
    if questions.is_empty() {
        return Err(Error::new("MCP 表单没有可填写字段"));
    }
    Ok(questions)
}
fn omission_id(id: &str) -> String {
    format!("velune:elicitation:omit:{id}")
}

fn required(schema: &Value, id: &str) -> bool {
    schema["required"]
        .as_array()
        .is_some_and(|fields| fields.iter().any(|field| field == id))
}
fn options(field: &Value) -> Vec<InteractionOption> {
    if field["type"] == "boolean" {
        return [("true", "是"), ("false", "否")]
            .into_iter()
            .map(|(id, label)| InteractionOption {
                id: format!("value:{id}"),
                label: label.into(),
                approval_kind: None,
            })
            .collect();
    }
    if let Some(values) = field["oneOf"].as_array() {
        return values
            .iter()
            .filter_map(|option| {
                let value = option["const"].as_str()?;
                Some(InteractionOption {
                    id: format!("value:{value}"),
                    label: option["title"].as_str().unwrap_or(value).into(),
                    approval_kind: None,
                })
            })
            .collect();
    }
    field["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, value)| {
            let value = value.as_str()?;
            Some(InteractionOption {
                id: format!("value:{value}"),
                label: field["enumNames"][index].as_str().unwrap_or(value).into(),
                approval_kind: None,
            })
        })
        .collect()
}
pub(crate) fn reply(params: &Value, reply: RuntimeInteractionReply) -> Result<Value> {
    let RuntimeInteractionReply::Answers { answers } = reply else {
        return match reply {
            RuntimeInteractionReply::Cancel => {
                Ok(json!({"action":"cancel","content":null,"_meta":null}))
            }
            _ => Err(Error::new("此 MCP 请求需要表单答案")),
        };
    };
    let schema = &params["requestedSchema"];
    let fields = schema["properties"]
        .as_object()
        .ok_or_else(|| Error::new("MCP 表单缺少字段"))?;
    let mut content = serde_json::Map::new();
    let mut seen = std::collections::BTreeSet::new();
    for answer in answers {
        let field = fields
            .get(&answer.question_id)
            .ok_or_else(|| Error::new("MCP 答案包含未知字段"))?;
        if !seen.insert(answer.question_id.clone()) || answer.values.len() != 1 {
            return Err(Error::new("MCP 表单字段需要一个答案"));
        }
        let raw = &answer.values[0];
        if !required(schema, &answer.question_id) && *raw == omission_id(&answer.question_id) {
            continue;
        }
        let choices = options(field);
        let value = choices
            .iter()
            .find(|option| option.id == *raw || option.label == *raw)
            .map(|option| option.id.strip_prefix("value:").unwrap_or(&option.id))
            .unwrap_or(raw);
        let value = match field["type"].as_str() {
            Some("boolean") => match value {
                "true" => json!(true),
                "false" => json!(false),
                _ => return Err(Error::new("请选择是或否")),
            },
            Some("integer") => json!(value.parse::<i64>().map_err(|_| Error::new("请输入整数"))?),
            Some("number") => {
                let number = value
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| Error::new("请输入有限数值"))?;
                json!(number)
            }
            Some("string") => {
                if !choices.is_empty()
                    && !choices
                        .iter()
                        .any(|option| option.id == format!("value:{value}"))
                {
                    return Err(Error::new("答案必须属于所列选项"));
                }
                let length = value.chars().count() as u64;
                if field["minLength"].as_u64().is_some_and(|n| length < n)
                    || field["maxLength"].as_u64().is_some_and(|n| length > n)
                {
                    return Err(Error::new("答案长度不符合表单要求"));
                }
                json!(value)
            }
            _ => return Err(Error::new("不支持的 MCP 字段类型")),
        };
        if let Some(number) = value.as_f64()
            && (field["minimum"].as_f64().is_some_and(|n| number < n)
                || field["maximum"].as_f64().is_some_and(|n| number > n))
        {
            return Err(Error::new("答案数值超出表单范围"));
        }
        content.insert(answer.question_id, value);
    }
    if fields
        .keys()
        .any(|id| required(schema, id) && !content.contains_key(id))
    {
        return Err(Error::new("请填写所有必填字段"));
    }
    Ok(json!({"action":"accept","content":content,"_meta":null}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_forms_preserve_types_and_reject_invalid_required_values() {
        let params = json!({"mode":"form","message":"Configure","requestedSchema":{"type":"object","required":["enabled","count","choice"],"properties":{
            "enabled":{"type":"boolean"},"count":{"type":"integer","minimum":1,"maximum":3},
            "choice":{"type":"string","oneOf":[{"const":"a","title":"Alpha"},{"const":"b","title":"Beta"}]},"note":{"type":"string"}}}});
        assert_eq!(questions(&params).unwrap().len(), 4);
        let answers = |count: &str| RuntimeInteractionReply::Answers {
            answers: vec![
                InteractionAnswer {
                    question_id: "enabled".into(),
                    values: vec!["是".into()],
                },
                InteractionAnswer {
                    question_id: "count".into(),
                    values: vec![count.into()],
                },
                InteractionAnswer {
                    question_id: "choice".into(),
                    values: vec!["value:b".into()],
                },
                InteractionAnswer {
                    question_id: "note".into(),
                    values: vec![omission_id("note")],
                },
            ],
        };
        assert_eq!(
            reply(&params, answers("2")).unwrap()["content"],
            json!({"enabled":true,"count":2,"choice":"b"})
        );
        assert!(reply(&params, answers("4")).is_err());
        assert_eq!(
            reply(&params, RuntimeInteractionReply::Cancel).unwrap()["action"],
            "cancel"
        );
        assert!(questions(&json!({"mode":"url"})).is_err());
        let optional = json!({"mode":"form","requestedSchema":{"type":"object","properties":{"note":{"type":"string"}}}});
        let answer = |value: String| InteractionAnswer {
            question_id: "note".into(),
            values: vec![value],
        };
        assert_eq!(
            reply(
                &optional,
                RuntimeInteractionReply::Answers {
                    answers: vec![answer("omit".into())]
                }
            )
            .unwrap()["content"]["note"],
            "omit"
        );
        assert!(
            reply(
                &optional,
                RuntimeInteractionReply::Answers {
                    answers: vec![answer(omission_id("note")), answer(omission_id("note"))]
                }
            )
            .is_err()
        );
    }
}
