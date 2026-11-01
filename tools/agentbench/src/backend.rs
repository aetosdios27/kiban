use crate::model::Memory;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::time::Duration;

pub use crate::api::{Backend, Result};

pub use crate::embedded::Embedded;

pub struct Helix {
    client: Client,
    endpoint: String,
}

impl Helix {
    pub fn new(endpoint: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()?,
            endpoint: endpoint.trim_end_matches('/').into(),
        })
    }

    fn query(&self, name: &str, params: &Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}/{name}", self.endpoint))
            .json(params)
            .send()?
            .error_for_status()?;
        let value: Value = response.json()?;
        if value.get("error").is_some() || value.get("errors").is_some() {
            return Err(format!("Helix application error: {value}").into());
        }
        Ok(value)
    }

    pub fn noop(&self) -> Result<()> {
        let value = self.query("Noop", &json!({}))?;
        if !value
            .as_object()
            .is_some_and(|o| o.values().any(|v| v == "agentbench-noop"))
        {
            return Err(format!("unexpected Noop response: {value}").into());
        }
        Ok(())
    }
}

impl Backend for Helix {
    fn prepare(&mut self) -> Result<()> {
        for agent in 0..crate::model::AGENTS {
            self.query("InitAgent", &json!({"agent": agent}))?;
        }
        Ok(())
    }
    fn put(&mut self, memory: &Memory, insert: bool) -> Result<()> {
        let (name, params) = if insert {
            ("Remember", serde_json::to_value(memory)?)
        } else {
            (
                "Update",
                json!({"memory_id": memory.memory_id, "revision": memory.revision, "text": memory.text}),
            )
        };
        let value = self.query(name, &params)?;
        if value["data"] != "ok" {
            return Err(format!("unexpected write acknowledgement: {value}").into());
        }
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let value = self.query("Recall", &json!({"memory_id": format!("memory-{id:020}")}))?;
        Ok(serde_json::from_value(value["memory"].clone())?)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let value = self.query("Recent", &json!({"agent": agent, "k": k}))?;
        Ok(serde_json::from_value(value["memories"].clone())?)
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}
