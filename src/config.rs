use serde::{Deserialize, Serialize};
use std::path::Path;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sources: Vec<String>,
    /// Optional filters to apply when subscribing to source relays
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<SubFilter>>,
    pub sinks: Vec<SinkConfig>,
    #[serde(default = "default_state_file")]
    pub state_file: String,
}

fn default_state_file() -> String {
    "nef_state.json".to_string()
}

/// A subscription filter matching REQ filter specification (NIP-01)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubFilter {
    /// List of pubkeys (authors) to match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,

    /// List of event kinds to match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<u16>>,

    /// Tag filters - use format like "#e", "#p", "#g" as keys
    #[serde(flatten)]
    pub tags: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkConfig {
    pub processor: ProcessorType,
    pub relays: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProcessorType {
    Passthrough,
    WaitProcessor,
    GeohashFilter {
        #[serde(default)]
        allowed_prefixes: Vec<String>,
    },
}

impl Config {
    pub async fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sources: vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.nostr.band".to_string(),
            ],
            filters: None,
            sinks: vec![
                SinkConfig {
                    processor: ProcessorType::Passthrough,
                    relays: vec!["wss://nos.lol".to_string()],
                },
            ],
            state_file: default_state_file(),
        }
    }
}