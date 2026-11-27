use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// The muffit daemon on this machine (docs/18).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Muffit {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token_file: String,
    /// Absent is on exactly when a token is found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The command a new session's terminal runs muffit's TUI with; `muffit` when empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tui: String,
}

pub const MUFFIT_URL: &str = "http://127.0.0.1:7891";

impl Muffit {
    pub fn tui_command(&self) -> &str {
        match self.tui.trim() {
            "" => "muffit",
            tui => tui,
        }
    }
}

/// Where an endpoint's url came from, said in the log so a node dialling the wrong daemon says
/// why it chose that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlFrom {
    Config,
    Muffit(PathBuf),
    Default,
}

impl fmt::Display for UrlFrom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config => f.write_str("[muffit] url"),
            Self::Muffit(path) => write!(f, "[server] in {}", path.display()),
            Self::Default => f.write_str("muffit's default port"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuffitEndpoint {
    pub url: String,
    pub token: String,
    pub from: UrlFrom,
}

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

impl Muffit {
    /// Where the daemon is and the token it takes, or `None` when this node does not talk to one.
    pub fn resolve(&self, home: &Path) -> Option<MuffitEndpoint> {
        self.resolve_with(home, &|name| {
            std::env::var(name).ok().filter(|v| !v.trim().is_empty())
        })
    }

    fn resolve_with(&self, home: &Path, env: Env) -> Option<MuffitEndpoint> {
        if self.enabled == Some(false) {
            return None;
        }
        let read = |path: PathBuf| {
            std::fs::read_to_string(path)
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };
        let token = Some(self.token.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                Some(&self.token_file)
                    .filter(|f| !f.is_empty())
                    .and_then(|f| read(PathBuf::from(f)))
            })
            .or_else(|| env("MUFFIT_SERVER_TOKEN").map(|t| t.trim().to_string()))
            .or_else(|| env("MUFFIT_HOME").and_then(|h| read(PathBuf::from(h).join("server.token"))))
            .or_else(|| read(home.join(".local/share/muffit/server.token")));
        let Some(token) = token else {
            if self.enabled == Some(true) {
                tracing::warn!("muffit is enabled but no token was found; set [muffit] token or token_file");
            }
            return None;
        };
        let (url, from) = match self.url.trim() {
            "" => match daemon_url(home, env) {
                Some((url, path)) => (url, UrlFrom::Muffit(path)),
                None => (MUFFIT_URL.to_string(), UrlFrom::Default),
            },
            url => (url.to_string(), UrlFrom::Config),
        };
        Some(MuffitEndpoint { url, token, from })
    }
}

/// Where muffit's own config says its daemon listens. A host with a second daemon on 7891 (a
/// container's) has its own moved off the default, and a node that dialled the default would be
/// refused by the wrong one. Muffit finds its config the way `user_config_path` does there.
fn daemon_url(home: &Path, env: Env) -> Option<(String, PathBuf)> {
    let path = match env("MUFFIT_CONFIG_FILE") {
        Some(file) => PathBuf::from(file),
        None => match env("XDG_CONFIG_HOME").filter(|dir| Path::new(dir).is_absolute()) {
            Some(dir) => PathBuf::from(dir).join("muffit/config.toml"),
            None => home.join(".config/muffit/config.toml"),
        },
    };
    let config: toml::Table = std::fs::read_to_string(&path).ok()?.parse().ok()?;
    let server = config.get("server")?.as_table()?;
    let host = server
        .get("host")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|h| !h.is_empty());
    let port = server
        .get("port")
        .and_then(toml::Value::as_integer)
        .and_then(|p| u16::try_from(p).ok());
    if host.is_none() && port.is_none() {
        return None;
    }
    let host = match host {
        None | Some("0.0.0.0" | "::" | "[::]") => "127.0.0.1".to_string(),
        Some(h) if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
        Some(h) => h.to_string(),
    };
    let default_port = MUFFIT_URL.rsplit(':').next().unwrap_or_default();
    let port = port.map_or_else(|| default_port.to_string(), |p| p.to_string());
    Some((format!("http://{host}:{port}"), path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn muffit_is_reached_with_the_first_token_found_and_only_then() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("token");
        std::fs::write(&file, "from-file\n").unwrap();
        let config = |token: &str, token_file: &str, enabled: Option<bool>| Muffit {
            url: String::new(),
            token: token.into(),
            token_file: token_file.into(),
            enabled,
            ..Muffit::default()
        };
        let file = file.display().to_string();
        let found = |m: Muffit| m.resolve_with(home.path(), &nothing).map(|e| (e.url, e.token));
        let default = |token: &str| Some((MUFFIT_URL.to_string(), token.to_string()));
        assert_eq!(found(config("given", &file, None)), default("given"));
        assert_eq!(found(config("", &file, None)), default("from-file"));
        assert_eq!(found(config("given", "", Some(false))), None);
        let elsewhere = Muffit {
            url: "http://10.0.0.2:7891".into(),
            ..config("given", "", None)
        };
        assert_eq!(found(elsewhere).unwrap().0, "http://10.0.0.2:7891");
        assert_eq!(found(config("", "", None)), None, "no token is no muffit");
        assert_eq!(found(config("", "", Some(true))), None);
        let share = home.path().join(".local/share/muffit");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::write(share.join("server.token"), "from-home").unwrap();
        assert_eq!(found(config("", "", None)), default("from-home"));
    }

    fn url_beside(muffit_config: Option<&str>, url: &str) -> (String, bool) {
        let home = tempfile::tempdir().unwrap();
        if let Some(text) = muffit_config {
            let dir = home.path().join(".config/muffit");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.toml"), text).unwrap();
        }
        let muffit = Muffit {
            url: url.into(),
            token: "t".into(),
            ..Muffit::default()
        };
        let endpoint = muffit.resolve_with(home.path(), &nothing).unwrap();
        (endpoint.url, matches!(endpoint.from, UrlFrom::Muffit(_)))
    }

    #[test]
    fn a_node_dials_where_muffits_own_config_says_its_daemon_listens() {
        let cases = [
            ("[server]\nport = 7892\n", "http://127.0.0.1:7892", true),
            (
                "[server]\nhost = \"0.0.0.0\"\nport = 9000\n",
                "http://127.0.0.1:9000",
                true,
            ),
            (
                "[server]\nhost = \"::1\"\nport = 7893\n",
                "http://[::1]:7893",
                true,
            ),
            ("[server]\nhost = \"10.0.0.5\"\n", "http://10.0.0.5:7891", true),
            ("[mesh]\nname = \"desktop\"\n", MUFFIT_URL, false),
            ("[server]\ntoken = \"x\"\n", MUFFIT_URL, false),
            ("[server]\nport = 70000\n", MUFFIT_URL, false),
            ("this is = = not toml", MUFFIT_URL, false),
        ];
        for (config, want, from_muffit) in cases {
            assert_eq!(
                url_beside(Some(config), ""),
                (want.to_string(), from_muffit),
                "{config}"
            );
        }
        assert_eq!(url_beside(None, ""), (MUFFIT_URL.to_string(), false));
        assert_eq!(
            url_beside(Some("[server]\nport = 7892\n"), "http://127.0.0.1:7891"),
            ("http://127.0.0.1:7891".to_string(), false),
            "an explicit [muffit] url wins"
        );
    }

    #[test]
    fn muffits_config_is_found_where_muffit_looks_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, port: u16| {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("[server]\nport = {port}\n")).unwrap();
        };
        write("home/.config/muffit/config.toml", 7001);
        write("xdg/muffit/config.toml", 7002);
        write("pinned.toml", 7003);
        let home = dir.path().join("home");
        let port = |env: &[(&str, String)]| {
            daemon_url(&home, &|name| {
                env.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
            })
            .map(|(url, _)| url)
        };
        let at = |p: &str| dir.path().join(p).display().to_string();
        assert_eq!(port(&[]).as_deref(), Some("http://127.0.0.1:7001"));
        assert_eq!(
            port(&[("XDG_CONFIG_HOME", at("xdg"))]).as_deref(),
            Some("http://127.0.0.1:7002")
        );
        assert_eq!(
            port(&[("XDG_CONFIG_HOME", "xdg".into())]).as_deref(),
            Some("http://127.0.0.1:7001"),
            "a relative XDG_CONFIG_HOME is ignored, as muffit ignores it"
        );
        assert_eq!(
            port(&[
                ("XDG_CONFIG_HOME", at("xdg")),
                ("MUFFIT_CONFIG_FILE", at("pinned.toml"))
            ])
            .as_deref(),
            Some("http://127.0.0.1:7003")
        );
    }
}
