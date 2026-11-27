use anyhow::{Result, bail};
use kampr_herdr::Herdr;
use std::os::unix::process::CommandExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SOURCE: &str = "kampr";
const ASKED: Duration = Duration::from_secs(2);

/// herdr restored this pane by running the argv it stored, and the keeper directories were on
/// tmpfs, so a reboot took the session and its log. The pane keeps its directory and gets a shell
/// rather than a command that exits.
pub fn into_shell(id: &str) -> Result<()> {
    eprintln!(
        "kampr: session {id} ended with the machine (its log was kept in memory); starting a shell \
         here instead"
    );
    forget_the_argv();
    let shell = kampr_core::login::shell();
    let error = std::process::Command::new(&shell).exec();
    bail!("starting {shell}: {error}")
}

/// herdr types the stored argv at every restart, and a report without one does not replace it
/// (#694); only another `resume_argv` does. The agent the report puts on the pane is released
/// again, since this pane is a shell, and the release needs a newer `seq` or herdr drops it (#695).
fn forget_the_argv() {
    let (Some(socket), Some(pane)) = (
        std::env::var_os("HERDR_SOCKET_PATH"),
        std::env::var("HERDR_PANE_ID").ok(),
    ) else {
        return;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return;
    };
    let herdr = Herdr::new(socket).with_timeout(ASKED);
    let seq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64);
    let nothing = ["true".to_string()];
    let _ = runtime.block_on(async {
        herdr
            .report_agent(&pane, SOURCE, SOURCE, "idle", seq, Some(&nothing))
            .await?;
        herdr
            .call::<serde_json::Value>(
                "pane.release_agent",
                serde_json::json!({ "pane_id": pane, "source": SOURCE, "agent": SOURCE, "seq": seq + 1 }),
            )
            .await
            .map(drop)
    });
}
