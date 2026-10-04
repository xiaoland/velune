//! Pi-only model projection metadata.
//!
//! These fields describe how Pi presents a routed model. They are adapter
//! metadata, not generic AI provider execution capabilities.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponsesCompat {
    pub supports_developer_role: bool,
    pub supports_mid_convo_system_messages: bool,
    pub supports_long_cache_retention: bool,
    pub supports_strict_mode: bool,
    #[serde(rename = "supportsOpenAIGrammarTools")]
    pub supports_open_ai_grammar_tools: bool,
    pub supports_additional_tools: bool,
    pub supports_tool_search: bool,
    pub supports_explicit_prompt_cache_mode: bool,
    pub supports_max_output_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PiModelProjection {
    pub reasoning_enabled: bool,
    #[serde(default)]
    pub thinking_level_map: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub responses_compat: Option<ResponsesCompat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResponsesConstraints {
    pub allowed_reasoning_efforts: Vec<String>,
}

impl PiModelProjection {
    pub fn supported_levels(&self, reasoning_levels: &[String]) -> Vec<String> {
        if !self.reasoning_enabled {
            return Vec::new();
        }
        reasoning_levels
            .iter()
            .filter(|level| {
                self.thinking_level_map
                    .get(*level)
                    .is_some_and(|value| value.is_some())
            })
            .cloned()
            .collect()
    }

    pub fn catalog_thinking_level_map(
        &self,
        reasoning_levels: &[String],
    ) -> BTreeMap<String, Option<String>> {
        ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
            .into_iter()
            .map(|level| {
                let value = if reasoning_levels.iter().any(|allowed| allowed == level) {
                    self.thinking_level_map.get(level).cloned().flatten()
                } else {
                    None
                };
                (level.to_owned(), value)
            })
            .collect()
    }

    pub fn native_responses_constraints(
        &self,
        reasoning_levels: &[String],
    ) -> NativeResponsesConstraints {
        let allowed_reasoning_efforts = if self.reasoning_enabled {
            reasoning_levels
                .iter()
                .filter_map(|level| self.thinking_level_map.get(level).cloned().flatten())
                .collect()
        } else {
            Vec::new()
        };
        NativeResponsesConstraints {
            allowed_reasoning_efforts,
        }
    }
}
