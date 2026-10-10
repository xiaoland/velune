use super::{Error, Result};
use agent_client_protocol_schema::v1 as wire;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Optional agent-owned UX category; config identity never depends on it.
    pub category: Option<String>,
    pub value: ConfigValue,
    pub choices: Vec<ConfigChoice>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub group_name: Option<String>,
}

pub(super) fn project(options: Vec<wire::SessionConfigOption>) -> Result<Vec<ConfigOption>> {
    let mut ids = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for option in options {
        let id = option.id.to_string();
        if !ids.insert(id.clone()) {
            return Err(Error::new("ACP 会话配置身份重复"));
        }
        let mut choices = Vec::new();
        let value = match option.kind {
            wire::SessionConfigKind::Select(select) => {
                match select.options {
                    wire::SessionConfigSelectOptions::Ungrouped(values) => {
                        choices.extend(
                            values
                                .into_iter()
                                .map(|choice| project_choice(choice, None, None)),
                        );
                    }
                    wire::SessionConfigSelectOptions::Grouped(groups) => {
                        for group in groups {
                            let id = group.group.to_string();
                            choices.extend(group.options.into_iter().map(|choice| {
                                project_choice(choice, Some(&id), Some(&group.name))
                            }));
                        }
                    }
                    _ => continue,
                }
                let mut values = std::collections::BTreeSet::new();
                if choices.iter().any(|choice| !values.insert(&choice.value)) {
                    return Err(Error::new("ACP 配置选项身份重复"));
                }
                ConfigValue::Select(select.current_value.to_string())
            }
            wire::SessionConfigKind::Boolean(value) => ConfigValue::Boolean(value.current_value),
            _ => continue,
        };
        result.push(ConfigOption {
            id,
            name: option.name,
            description: option.description,
            category: option
                .category
                .and_then(|category| serde_json::to_value(category).ok())
                .and_then(|category| category.as_str().map(str::to_owned)),
            value,
            choices,
        });
    }
    Ok(result)
}
fn project_choice(
    choice: wire::SessionConfigSelectOption,
    group: Option<&str>,
    group_name: Option<&str>,
) -> ConfigChoice {
    ConfigChoice {
        value: choice.value.to_string(),
        name: choice.name,
        description: choice.description,
        group: group.map(str::to_owned),
        group_name: group_name.map(str::to_owned),
    }
}
