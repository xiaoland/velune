use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub source_provider_id: String,
    pub source_provider_name: String,
    pub model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}
