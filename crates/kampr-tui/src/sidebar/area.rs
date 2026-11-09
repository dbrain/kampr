use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    #[default]
    Terminals,
    Containers,
    Muffit,
}

impl Area {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Terminals => "›_",
            Self::Containers => "▣",
            Self::Muffit => "◉",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tab {
    pub area: Area,
    pub count: usize,
}

pub fn step(tabs: &[Tab], from: Area, by: i32) -> Option<Area> {
    let at = tabs.iter().position(|t| t.area == from)? as i32;
    let n = tabs.len() as i32;
    Some(tabs[(at + by).rem_euclid(n) as usize].area)
}

/// An area string a newer build wrote reads as the default rather than failing the whole file.
fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Area, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(serde_json::from_value(serde_json::Value::String(raw)).unwrap_or_default())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Host {
    #[serde(deserialize_with = "lenient")]
    pub area: Area,
    pub stopped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hosts(BTreeMap<String, Host>);

impl Hosts {
    pub fn area(&self, node: &str) -> Area {
        self.0.get(node).map(|h| h.area).unwrap_or_default()
    }

    pub fn show(&mut self, node: &str, area: Area) {
        self.0.entry(node.to_string()).or_default().area = area;
    }

    pub fn stopped(&self, node: &str) -> bool {
        self.0.get(node).is_some_and(|h| h.stopped)
    }

    pub fn flip_stopped(&mut self, node: &str) {
        let host = self.0.entry(node.to_string()).or_default();
        host.stopped = !host.stopped;
    }
}
