use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Stop,
    Reload,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PactlEvent {
    pub event: String,
    pub on: String,
    #[serde(default)]
    pub index: Option<u64>,
}

impl PactlEvent {
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }

    pub fn key(&self) -> (&str, &str, Option<u64>) {
        (self.on.as_str(), self.event.as_str(), self.index)
    }

    pub fn formatted(&self) -> String {
        match self.index {
            Some(idx) => format!("Event '{}' on {} #{}", self.event, self.on, idx),
            None => format!("Event '{}' on {}", self.event, self.on),
        }
    }
}

pub fn is_interesting(event: &PactlEvent) -> bool {
    matches!(event.on.as_str(), "card" | "server")
}
