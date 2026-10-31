//! Workspace names, kept apart from the sessions that name a workspace by id, so renaming one is
//! one write rather than one per session.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE: &str = "workspaces.json";
pub const WORKSPACE_PREFIX: &str = "kw:";

#[derive(Debug, Default)]
pub(crate) struct Workspaces {
    path: PathBuf,
    names: BTreeMap<String, String>,
}

impl Workspaces {
    pub(crate) fn load(root: &Path) -> Self {
        let path = root.join(FILE);
        let names = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, names }
    }

    fn save(&self) -> Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&self.names)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).with_context(|| format!("replacing {}", self.path.display()))
    }

    pub(crate) fn name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// A new workspace even where one already has the name, as herdr makes one per create.
    pub(crate) fn create(&mut self, name: &str) -> Result<String> {
        let base = slug(name);
        let id = (1..)
            .map(|n| match n {
                1 => base.clone(),
                n => format!("{base}-{n}"),
            })
            .find(|id| !self.names.contains_key(id))
            .expect("an unused id");
        self.names.insert(id.clone(), name.to_string());
        self.save()?;
        Ok(id)
    }

    /// An id, or a name: the first workspace with it, made when there is none.
    pub(crate) fn resolve(&mut self, wanted: &str) -> Result<String> {
        let id = wanted.strip_prefix(WORKSPACE_PREFIX).unwrap_or(wanted);
        if self.names.contains_key(id) {
            return Ok(id.to_string());
        }
        match self.names.iter().find(|(_, name)| *name == wanted) {
            Some((id, _)) => Ok(id.clone()),
            None => self.create(wanted),
        }
    }

    pub(crate) fn rename(&mut self, id: &str, name: &str) -> Result<bool> {
        match self.names.get_mut(id) {
            Some(have) => {
                *have = name.to_string();
                self.save()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '-',
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    match slug.is_empty() {
        true => "workspace".to_string(),
        false => slug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_resolves_to_its_workspace_and_a_create_never_reuses_one() {
        let root = tempfile::tempdir().unwrap();
        let mut spaces = Workspaces::load(root.path());
        let api = spaces.create("My API").unwrap();
        assert_eq!(api, "my-api");
        assert_eq!(spaces.create("My API").unwrap(), "my-api-2");
        assert_eq!(spaces.resolve("My API").unwrap(), "my-api");
        assert_eq!(spaces.resolve("kw:my-api-2").unwrap(), "my-api-2");
        assert!(spaces.rename("my-api", "backend").unwrap());
        assert_eq!(Workspaces::load(root.path()).name("my-api"), Some("backend"));
        assert_eq!(spaces.resolve("新").unwrap(), "workspace");
    }
}
