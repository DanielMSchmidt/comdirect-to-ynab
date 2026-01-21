use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub last_synced_at: Option<DateTime<Utc>>,
    pub reference_occurrences: HashMap<String, ReferenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub date: NaiveDate,
    pub amount_milli: i64,
    pub occurrence: u32,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read state at {}", path.display()))?;
        let state = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse state at {}", path.display()))?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create state directory {}", parent.display())
            })?;
        }
        let contents = serde_json::to_string_pretty(self).context("failed to serialize state")?;
        fs::write(path, contents)
            .with_context(|| format!("failed to write state to {}", path.display()))?;
        Ok(())
    }

    pub fn prune(&mut self, lookback_days: i64) {
        let cutoff = Utc::now().date_naive() - Duration::days(lookback_days.max(1));
        self.reference_occurrences
            .retain(|_, entry| entry.date >= cutoff);
    }

    pub fn build_counters(&self) -> HashMap<String, u32> {
        let mut counters = HashMap::new();
        for entry in self.reference_occurrences.values() {
            let key = format!("{}|{}", entry.date, entry.amount_milli);
            let current = counters.get(&key).copied().unwrap_or(0);
            if entry.occurrence > current {
                counters.insert(key, entry.occurrence);
            }
        }
        counters
    }
}
