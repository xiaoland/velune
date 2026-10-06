//! Explicit public-directory retrieval. Entries remain scoped to their source
//! provider and are suggestions for editable templates, never configured models.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

const MAX_BYTES: usize = 16 * 1024 * 1024;

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

/// Read the models.dev provider-scoped API format. Missing and zero limits mean
/// unknown. A reasoning boolean does not define supported reasoning levels.
pub fn parse_models_dev(bytes: &[u8]) -> Result<Vec<CatalogModel>, Error> {
    #[derive(Deserialize)]
    struct Provider {
        id: String,
        name: String,
        models: BTreeMap<String, Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
        name: String,
        #[serde(default)]
        limit: Limits,
        #[serde(default)]
        reasoning_options: Vec<ReasoningOption>,
    }
    #[derive(Deserialize)]
    struct ReasoningOption {
        #[serde(rename = "type")]
        kind: String,
        values: Option<Vec<Option<String>>>,
    }
    #[derive(Default, Deserialize)]
    struct Limits {
        context: Option<u32>,
        output: Option<u32>,
    }
    if bytes.len() > MAX_BYTES {
        return Err(Error::invalid("公开模型目录超过大小限制"));
    }
    let providers: BTreeMap<String, Provider> = serde_json::from_slice(bytes)
        .map_err(|_| Error::invalid("models.dev 目录格式无效，请稍后重新拉取"))?;
    let mut entries = Vec::new();
    for provider in providers.into_values() {
        if provider.id.trim().is_empty() || provider.name.trim().is_empty() {
            return Err(Error::invalid("公开模型目录的提供商标识无效"));
        }
        for model in provider.models.into_values() {
            if model.id.trim().is_empty() || model.name.trim().is_empty() {
                return Err(Error::invalid("公开模型目录的模型标识无效"));
            }
            entries.push(CatalogModel {
                source_provider_id: provider.id.clone(),
                source_provider_name: provider.name.clone(),
                model_id: model.id,
                name: model.name,
                context_window: model.limit.context.filter(|value| *value > 0),
                max_output_tokens: model.limit.output.filter(|value| *value > 0),
                reasoning_levels: model
                    .reasoning_options
                    .into_iter()
                    .find(|option| option.kind == "effort")
                    .and_then(|option| option.values)
                    .map(|values| values.into_iter().flatten().collect::<Vec<_>>())
                    .filter(|values| {
                        !values.is_empty()
                            && values.iter().all(|value| {
                                !value.trim().is_empty() && !value.chars().any(char::is_control)
                            })
                    }),
            });
        }
    }
    entries.sort_by(|a, b| {
        (&a.source_provider_name, &a.name, &a.model_id).cmp(&(
            &b.source_provider_name,
            &b.name,
            &b.model_id,
        ))
    });
    Ok(entries)
}

/// Synchronous operation: platform callers must use a worker queue.
/// Fetch only the fixed public HTTPS endpoint, without provider credentials,
/// redirects or background refresh. Configuration is not read or modified.
pub fn fetch_models_dev() -> Result<Vec<CatalogModel>, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::invalid("无法创建公开目录请求"))?
        .block_on(async {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| Error::invalid("无法创建公开目录请求"))?;
            let mut response = client
                .get("https://models.dev/api.json")
                .send()
                .await
                .map_err(|_| Error::invalid("无法连接 models.dev，请检查网络后重试"))?;
            if !response.status().is_success() {
                return Err(Error::invalid("models.dev 返回错误，请稍后重新拉取"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| Error::invalid("models.dev 下载中断，请重新拉取"))?
            {
                if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                    return Err(Error::invalid("公开模型目录超过大小限制"));
                }
                bytes.extend_from_slice(&chunk);
            }
            parse_models_dev(&bytes)
        })
}
