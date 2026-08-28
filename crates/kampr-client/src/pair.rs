use crate::profile::{ClientConfig, Profile};
use crate::resolve::hostname;
use std::path::Path;
use std::time::Duration;

const PAIR_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("reaching {0}: {1}")]
    Unreachable(String, String),
    #[error("{0} refused the code: {1}")]
    Refused(String, String),
    #[error("{0} answered a pairing with no token in it")]
    NoToken(String),
    #[error(transparent)]
    Config(#[from] crate::profile::ConfigError),
}

pub struct Paired {
    pub name: String,
    pub origin: String,
    pub device: String,
}

/// Redeems a pairing code against a herd on another machine and saves the token as a profile, so
/// a bare `kampr` on this machine opens it with no arguments and no prompt.
///
/// The credential is the same device token a browser gets from the same route; there is no
/// second, weaker way in for a terminal.
pub async fn pair(
    config_dir: &Path,
    origin: &str,
    code: &str,
    profile: Option<&str>,
    device_name: Option<&str>,
) -> Result<Paired, PairError> {
    let origin = origin.trim().trim_end_matches('/').to_string();
    let device = device_name
        .map(str::to_string)
        .unwrap_or_else(|| format!("cli@{}", hostname()));
    let client = reqwest::Client::builder()
        .timeout(PAIR_TIMEOUT)
        .build()
        .map_err(|e| PairError::Unreachable(origin.clone(), e.to_string()))?;
    let response = client
        .post(format!("{origin}/auth/pair"))
        .json(&serde_json::json!({ "code": code.trim(), "device_name": device }))
        .send()
        .await
        .map_err(|e| PairError::Unreachable(origin.clone(), e.to_string()))?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        let said = body["error"].as_str().unwrap_or("no reason given").to_string();
        return Err(PairError::Refused(origin, said));
    }
    let token = body["token"]
        .as_str()
        .ok_or_else(|| PairError::NoToken(origin.clone()))?
        .to_string();

    let name = profile.map(str::to_string).unwrap_or_else(|| host_of(&origin));
    let mut config = ClientConfig::load(config_dir)?;
    config.profiles.insert(
        name.clone(),
        Profile {
            origin: origin.clone(),
            token,
        },
    );
    // The one just paired is the one meant, and a machine with two saved herds and no default
    // would otherwise open neither.
    config.default = Some(name.clone());
    config.save(config_dir)?;
    Ok(Paired { name, origin, device })
}

fn host_of(origin: &str) -> String {
    origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .split(['/', ':'])
        .next()
        .filter(|host| !host.is_empty())
        .unwrap_or("herd")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_named_after_the_host_it_reaches() {
        assert_eq!(host_of("https://kampr.example.com"), "kampr.example.com");
        assert_eq!(host_of("http://192.168.1.24:8790"), "192.168.1.24");
        assert_eq!(host_of("kampr.example.com/"), "kampr.example.com");
    }
}
