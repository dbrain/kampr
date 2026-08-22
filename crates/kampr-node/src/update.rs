use crate::config::Config;
use crate::state::BUILD;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::debug;

/// How long a good answer stands before the node asks again.
///
/// **Once a day, and held on disk.** A release check that rode the herd rebuild would be a
/// request every few seconds on a busy host, and one held only in memory would be a request per
/// restart under a supervisor that is crash-looping. Neither is a thing to point at GitHub from
/// every machine an operator owns.
const SETTLED: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a *failed* check waits. Shorter than a day because the overwhelmingly common failure
/// is a laptop that was not on a network yet when its node started, and a day of silence for a
/// thirty-second outage is a worse answer than an hour.
const RETRY: Duration = Duration::from_secs(60 * 60);

const TIMEOUT: Duration = Duration::from_secs(10);

fn cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join("update.json")
}

/// The last answer and when it was got, so the cadence survives a restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    /// The newest release tag GitHub named, `v` and all — kept even when the last attempt failed,
    /// because a stale answer is better than none and its age is recorded beside it.
    pub latest: Option<String>,
    pub checked_at: u64,
    /// Whether the last attempt got an answer, which is the whole difference between the two
    /// cadences.
    pub ok: bool,
}

impl Cache {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Written beside and renamed, because the node's own check and a `kampr update --check` at
    /// a shell write the same file and a torn one would read as "never checked".
    fn save(&self, path: &Path) {
        let staged = path.with_extension("json.new");
        let wrote = serde_json::to_string(self)
            .map_err(std::io::Error::other)
            .and_then(|text| std::fs::write(&staged, text))
            .and_then(|()| std::fs::rename(&staged, path));
        if let Err(e) = wrote {
            debug!(path = %path.display(), error = %e, "could not cache the release check");
        }
    }

    /// How long until the next attempt is due; zero when it is due now.
    fn due_in(&self) -> Duration {
        let interval = if self.ok { SETTLED } else { RETRY };
        let age = now().saturating_sub(self.checked_at);
        interval.saturating_sub(Duration::from_secs(age))
    }

    /// What this node should put on the wire, given what it is running.
    pub fn against(&self, build: &str) -> Option<String> {
        supersedes(build, self.latest.as_deref()?)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The released version, when it is newer than what is running here — and nothing at all when it
/// is not, when either side does not parse, or when this build is ahead of the last release,
/// which is every working copy between two tags.
///
/// Build metadata is ignored, per semver: `0.1.0+abc1234` and `0.1.0` are the same release.
pub fn supersedes(build: &str, tag: &str) -> Option<String> {
    let tag = tag.trim().trim_start_matches('v');
    let running = semver::Version::parse(build.trim().trim_start_matches('v')).ok()?;
    let released = semver::Version::parse(tag).ok()?;
    (released > running).then(|| tag.to_string())
}

/// Starts the once-a-day release check, or does not.
///
/// The receiver carries the answer for the herd model to hand out. `None` covers every case where
/// there is nothing to say — current, unreachable, unparsable, switched off — because a client
/// renders all four the same way, which is not at all.
pub fn start(config: &Config, state_dir: &Path) -> (watch::Receiver<Option<String>>, Vec<JoinHandle<()>>) {
    let (tx, rx) = watch::channel(None);
    if !config.update.check {
        tracing::info!("release discovery is off; this node will not ask GitHub for a version");
        return (rx, Vec::new());
    }
    let url = config.update.latest_release_url();
    let path = cache_path(state_dir);
    (rx, vec![tokio::spawn(poll(url, path, tx))])
}

/// The one place the off switch is read on the way *out*, so the wire, `kampr status` and
/// `kampr update --check` cannot disagree about whether this node has anything to say.
///
/// It beats a cache that is still on disk from before the switch was thrown: off means the node
/// has nothing to say, not that it stops asking and keeps reporting the last answer.
pub fn available(config: &Config, state_dir: &Path) -> Option<String> {
    config
        .update
        .check
        .then(|| Cache::load(&cache_path(state_dir)).against(BUILD))
        .flatten()
}

fn refuse_if_off(config: &Config) -> anyhow::Result<()> {
    match config.update.check {
        true => Ok(()),
        false => anyhow::bail!(
            "release discovery is off for this node: [update] check = false in its config, and \
             nothing here will go round that. Set it to true, or read the release notes at \
             https://github.com/{}/releases",
            config.update.repo
        ),
    }
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!("kampr/{BUILD}"))
        .timeout(TIMEOUT)
        .build()?)
}

/// One answer, now, honouring the same cadence and the same cache the node's own task uses — so
/// asking at a shell neither costs a request the node already paid for nor resets its clock.
pub async fn check(config: &Config, state_dir: &Path) -> anyhow::Result<Cache> {
    refuse_if_off(config)?;
    let path = cache_path(state_dir);
    let cache = Cache::load(&path);
    if !cache.due_in().is_zero() {
        return Ok(cache);
    }
    let url = config.update.latest_release_url();
    match ask(&client()?, &url).await {
        Ok(tag) => {
            let fresh = Cache {
                latest: Some(tag),
                checked_at: now(),
                ok: true,
            };
            fresh.save(&path);
            Ok(fresh)
        }
        Err(e) => {
            Cache {
                ok: false,
                checked_at: now(),
                ..cache
            }
            .save(&path);
            Err(e.context(format!("asking {url} for the latest release")))
        }
    }
}

async fn poll(url: String, path: PathBuf, answer: watch::Sender<Option<String>>) {
    let client = match client() {
        Ok(client) => client,
        Err(e) => {
            debug!(error = %e, "no http client for the release check");
            return;
        }
    };
    loop {
        let mut cache = Cache::load(&path);
        publish(&answer, &cache);
        let due = cache.due_in();
        if !due.is_zero() {
            tokio::time::sleep(due).await;
            continue;
        }
        match ask(&client, &url).await {
            Ok(tag) => {
                cache.latest = Some(tag);
                cache.ok = true;
            }
            // Silence, not an error: a node with no route out is not a node with a problem, and
            // nothing here is allowed to reach the operator's herd view.
            Err(e) => {
                debug!(%url, error = %e, "the release check did not get an answer");
                cache.ok = false;
            }
        }
        cache.checked_at = now();
        cache.save(&path);
        publish(&answer, &cache);
        tokio::time::sleep(cache.due_in()).await;
    }
}

fn publish(answer: &watch::Sender<Option<String>>, cache: &Cache) {
    let available = cache.against(BUILD);
    answer.send_if_modified(|held| {
        let moved = *held != available;
        *held = available;
        moved
    });
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

async fn ask(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<Release>().await?.tag_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_strictly_newer_release_is_worth_saying() {
        let cases = [
            ("0.1.0", "v0.1.2", Some("0.1.2")),
            ("0.1.0", "0.1.2", Some("0.1.2")),
            ("0.1.0", "v0.1.0", None),
            // Build metadata is not a version. A node built from the tag reports the tag plus a
            // commit, and telling it to update to itself is the loudest possible way to be wrong.
            ("0.1.0+abc1234", "v0.1.0", None),
            ("0.1.0+abc1234", "v0.1.1", Some("0.1.1")),
            // A working copy between two tags is ahead of the last release, not behind it.
            ("0.2.0", "v0.1.9", None),
            ("0.1.0", "v1.0.0", Some("1.0.0")),
            // A prerelease is older than the release it leads to, and newer than the one before.
            ("0.2.0-rc.1", "v0.2.0", Some("0.2.0")),
            ("0.2.0", "v0.2.0-rc.2", None),
            // Neither side parsing is not an error to report; it is a thing not to say.
            ("dirty-worktree", "v9.9.9", None),
            ("0.1.0", "nightly", None),
            ("0.1.0", "", None),
        ];
        for (build, tag, want) in cases {
            assert_eq!(
                supersedes(build, tag).as_deref(),
                want,
                "running {build}, latest {tag}"
            );
        }
    }

    #[test]
    fn a_good_answer_stands_for_a_day_and_a_failed_one_for_an_hour() {
        let fresh = Cache {
            latest: Some("v0.1.2".into()),
            checked_at: now(),
            ok: true,
        };
        assert!(fresh.due_in() > SETTLED - Duration::from_secs(5));
        let failed = Cache {
            ok: false,
            ..fresh.clone()
        };
        assert!(failed.due_in() <= RETRY, "a failure waited longer than the retry");
        assert!(failed.due_in() > RETRY - Duration::from_secs(5));
        let stale = Cache {
            checked_at: now() - SETTLED.as_secs() - 1,
            ..fresh.clone()
        };
        assert!(stale.due_in().is_zero(), "a day-old answer never came due again");
        // An empty cache is due immediately, or a node that has never asked never would.
        assert!(Cache::default().due_in().is_zero());
    }

    /// The one test that reaches the internet, so it is opt-in: `cargo test -p kampr-node
    /// --lib -- --ignored`. Everything else drives a stub, and a stub only ever proves the node
    /// agrees with itself about a shape GitHub was never asked to confirm.
    #[tokio::test]
    #[ignore = "reaches api.github.com"]
    async fn the_real_github_answers_the_shape_this_parses() {
        let config = Config::bootstrap("probe");
        let client = reqwest::Client::builder()
            .user_agent(format!("kampr/{BUILD}"))
            .timeout(TIMEOUT)
            .build()
            .expect("a client");
        let tag = ask(&client, &config.update.latest_release_url())
            .await
            .expect("api.github.com answered");
        println!("api.github.com named {tag}; this build is {BUILD}");
        assert!(tag.starts_with('v'), "the tag was {tag}");
        assert!(
            semver::Version::parse(tag.trim_start_matches('v')).is_ok(),
            "the published tag {tag} is not a version this can compare against"
        );
    }

    /// The last good answer outlives a failed attempt: a laptop that goes offline should keep
    /// saying what it knew, not forget it.
    #[test]
    fn a_failed_check_keeps_the_answer_it_already_had() {
        let dir = tempfile::tempdir().expect("a dir");
        let path = cache_path(dir.path());
        Cache {
            latest: Some("v9.9.9".into()),
            checked_at: 1,
            ok: true,
        }
        .save(&path);
        let mut reloaded = Cache::load(&path);
        assert_eq!(reloaded.latest.as_deref(), Some("v9.9.9"));
        reloaded.ok = false;
        reloaded.save(&path);
        assert_eq!(Cache::load(&path).latest.as_deref(), Some("v9.9.9"));
        assert_eq!(Cache::load(&path).against("0.1.0").as_deref(), Some("9.9.9"));
    }
}
