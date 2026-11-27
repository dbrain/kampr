use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use kampr_core::conversation::Stored;
use kampr_proc::Table;
use serde_json::Value;

use crate::config::Config;

/// How many stored conversations a listing returns, newest first.
const LISTED: usize = 200;
/// How much of a transcript is read to name it.
const HEAD: u64 = 64 * 1024;

/// Where Claude keeps what it writes about its sessions: `<config>/projects/<slug>/<id>.jsonl`
/// for a transcript and `<config>/sessions/<pid>.json` for a process that holds one open.
#[derive(Clone)]
pub(crate) struct Store {
    config_dir: Option<PathBuf>,
}

impl Store {
    pub fn new(config: &Config) -> Self {
        Self {
            config_dir: config
                .config_dir()
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))),
        }
    }

    fn projects(&self) -> Option<PathBuf> {
        self.config_dir.as_ref().map(|d| d.join("projects"))
    }

    /// The transcript a Claude session id writes. The slug is derived from a cwd that is not
    /// always at hand, so the id is looked for in every project.
    pub fn find(&self, id: &str) -> Option<PathBuf> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        let name = format!("{id}.jsonl");
        std::fs::read_dir(self.projects()?)
            .ok()?
            .flatten()
            .map(|project| project.path().join(&name))
            .find(|candidate| candidate.is_file())
    }

    /// The one-opener rule (#655): a session id that a live process other than `own` holds open
    /// belongs to that process, and a second writer forks the transcript silently. Returns the
    /// process.
    pub fn foreign_opener(&self, id: &str, own: Option<u32>) -> Option<u32> {
        let table = kampr_proc::here();
        std::fs::read_dir(self.config_dir.as_ref()?.join("sessions"))
            .ok()?
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str::<Value>(&std::fs::read_to_string(e.path()).ok()?).ok())
            .filter(|meta| meta.get("sessionId").and_then(Value::as_str) == Some(id))
            .filter_map(|meta| u32::try_from(meta.get("pid")?.as_u64()?).ok())
            .find(|pid| Some(*pid) != own && table.argv(*pid).is_some())
    }

    /// The conversations on disk that could be resumed, newest first, in `cwd` when one is
    /// named. `skip` is the ones a process holds open already.
    pub fn stored(&self, cwd: Option<&Path>, skip: &HashSet<String>) -> Vec<Stored> {
        let Some(projects) = self.projects() else {
            return Vec::new();
        };
        let mut files: Vec<(SystemTime, String, PathBuf)> = std::fs::read_dir(projects)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten().flatten())
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .filter_map(|e| {
                let id = e.path().file_stem()?.to_str()?.to_string();
                Some((e.metadata().ok()?.modified().ok()?, id, e.path()))
            })
            .filter(|(_, id, _)| !skip.contains(id))
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        files
            .into_iter()
            .map(|(_, id, path)| {
                let (found_cwd, title) = head_of(&path);
                Stored {
                    id,
                    cwd: found_cwd,
                    title,
                }
            })
            .filter(|s| cwd.is_none_or(|want| s.cwd.as_deref() == Some(want)))
            .take(LISTED)
            .collect()
    }
}

pub(crate) fn cwd_of(transcript: &Path) -> Option<PathBuf> {
    head_of(transcript).0
}

/// What a transcript's first lines say of where it ran and what it was for.
fn head_of(path: &Path) -> (Option<PathBuf>, Option<String>) {
    let mut head = String::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(HEAD).read_to_string(&mut head);
    }
    let lines = head.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok());
    let (mut cwd, mut title) = (None, None);
    for line in lines {
        if cwd.is_none() {
            cwd = line.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        if title.is_none() && line.get("type").and_then(Value::as_str) == Some("user") {
            title = prompt_text(&line);
        }
        if cwd.is_some() && title.is_some() {
            break;
        }
    }
    (cwd, title)
}

fn prompt_text(line: &Value) -> Option<String> {
    if line.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let content = line.pointer("/message/content")?;
    let text = content.as_str().or_else(|| {
        content
            .as_array()?
            .iter()
            .find_map(|b| b.get("text").and_then(Value::as_str))
    })?;
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.chars().take(80).collect())
}

/// The Claude session a keeper session runs, read off its launch line.
pub(crate) fn claude_id(argv: &[String]) -> Option<&str> {
    argv.windows(2)
        .find(|w| w[0] == "--session-id" || w[0] == "-r")
        .map(|w| w[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> Store {
        Store {
            config_dir: Some(dir.to_path_buf()),
        }
    }

    fn transcript(dir: &Path, project: &str, id: &str, lines: &[&str]) {
        let project = dir.join("projects").join(project);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn a_transcript_is_found_by_its_session_id_in_any_project() {
        let dir = tempfile::tempdir().unwrap();
        transcript(dir.path(), "-w-a", "id-a", &["{}"]);
        transcript(dir.path(), "-w-b", "id-b", &["{}"]);
        let store = store(dir.path());
        assert!(store.find("id-b").unwrap().ends_with("-w-b/id-b.jsonl"));
        assert!(store.find("nope").is_none());
        assert!(store.find("../projects/-w-a/id-a").is_none());
    }

    #[test]
    fn stored_conversations_are_ids_with_their_cwd_and_first_prompt() {
        let dir = tempfile::tempdir().unwrap();
        transcript(
            dir.path(),
            "-w-a",
            "11111111-aaaa",
            &[
                r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>x"}}"#,
                r#"{"type":"user","cwd":"/w/a","message":{"content":"fix the build"}}"#,
            ],
        );
        transcript(
            dir.path(),
            "-w-b",
            "22222222-bbbb",
            &[r#"{"type":"user","cwd":"/w/b","message":{"content":[{"type":"text","text":"hello"}]}}"#],
        );
        let store = store(dir.path());

        let all = store.stored(None, &HashSet::new());
        let a = all
            .iter()
            .find(|s| s.id == "11111111-aaaa")
            .expect("a stored id, not a file name");
        assert_eq!(a.cwd.as_deref(), Some(Path::new("/w/a")));
        assert_eq!(a.title.as_deref(), Some("fix the build"));

        let here = store.stored(Some(Path::new("/w/b")), &HashSet::new());
        assert_eq!(
            here.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["22222222-bbbb"]
        );

        let skipped = store.stored(None, &HashSet::from(["11111111-aaaa".to_string()]));
        assert!(skipped.iter().all(|s| s.id != "11111111-aaaa"));
    }

    #[test]
    fn a_session_a_live_process_holds_open_belongs_to_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        let me = std::process::id();
        let held = |pid: u32, id: &str| {
            std::fs::write(
                dir.path().join("sessions").join(format!("{pid}.json")),
                format!(r#"{{"pid":{pid},"sessionId":"{id}"}}"#),
            )
            .unwrap();
        };
        held(me, "held-id");
        held(4_000_000, "dead-id");
        let store = store(dir.path());
        assert_eq!(store.foreign_opener("held-id", None), Some(me));
        assert_eq!(
            store.foreign_opener("held-id", Some(me)),
            None,
            "its own child is not a second opener"
        );
        assert_eq!(
            store.foreign_opener("dead-id", None),
            None,
            "a stale file names nobody"
        );
        assert_eq!(store.foreign_opener("free-id", None), None);
    }

    #[test]
    fn a_keeper_sessions_claude_id_is_read_off_its_launch_line() {
        let argv = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            claude_id(&argv(&["claude", "-p", "--session-id", "u-1"])),
            Some("u-1")
        );
        assert_eq!(claude_id(&argv(&["claude", "-p", "-r", "u-2"])), Some("u-2"));
        assert_eq!(claude_id(&argv(&["claude", "-p"])), None);
    }
}
