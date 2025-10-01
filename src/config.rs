use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sources: Vec<String>,
    pub sinks: Vec<SinkConfig>,
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
            sinks: vec![
                SinkConfig {
                    processor: ProcessorType::Passthrough,
                    relays: vec!["wss://nos.lol".to_string()],
                },
            ],
        }
    }
}