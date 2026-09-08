pub mod control;
pub mod locate;
pub mod model;
pub mod observe;
pub mod rpc;

pub use control::{Controller, HOLD_LIMIT};
pub use locate::{Found, Origin, Search};
pub use model::{
    AgentStatus, Command, ForegroundProcess, Hit, Matches, Pane, ProcessInfo, Snapshot, SnapshotReply,
};
pub use observe::{Observer, StreamEvent};
pub use rpc::{Herdr, RpcError, Sub};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// What herdr says it is holding after an `agent.view.*` call: whether a view is active, and the
/// `source` and `label` it was set with. The sort is not in it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct AgentView {
    pub active: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

impl Herdr {
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let r: SnapshotReply = self.call("session.snapshot", serde_json::json!({})).await?;
        Ok(r.snapshot)
    }

    /// The processes inside a pane.
    ///
    /// **The pane record carries no pid**, so this call is the whole of what a node can learn
    /// about which process a pane is running — and the working directory it does carry names a
    /// project, not a session.
    pub async fn process_info(&self, pane_id: &str) -> Result<model::ProcessInfo> {
        let r: model::ProcessInfoReply = self
            .call("pane.process_info", serde_json::json!({ "pane_id": pane_id }))
            .await?;
        Ok(r.process_info)
    }

    /// Reports a name Kampr computed for a pane into herdr's own metadata table.
    ///
    /// **`ok` here means well-formed, never applied** (probe #295). A `seq` older than the one
    /// this source last sent is dropped silently and still answered `ok`, and the record is
    /// per-source under last-writer-wins, so the only honest confirmation is [`Self::pane_title`]
    /// read back afterwards.
    pub async fn report_metadata(
        &self,
        pane_id: &str,
        source: &str,
        title: &str,
        tokens: &BTreeMap<String, String>,
        seq: u64,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .call(
                "pane.report_metadata",
                serde_json::json!({
                    "pane_id": pane_id,
                    "source": source,
                    "title": title,
                    "tokens": tokens,
                    "seq": seq,
                }),
            )
            .await?;
        Ok(())
    }

    /// Shapes herdr's **own** agents sidebar, at whoever's desk this session belongs to.
    ///
    /// Sortable fields are the tokens a source reported plus exactly two builtins, `agent` and
    /// `status` — nothing else is accepted, and there is no builtin for `title`. So a sort on a
    /// name Kampr computed only means anything once that name has been reported as a *token*,
    /// which is [`Self::report_metadata`]'s job and is itself behind a setting.
    ///
    /// `label` replaces the sort-mode word in the sidebar's section header and herdr refuses one
    /// that is empty or past 32 characters. The reply echoes `active`, `source` and `label` and
    /// says **nothing about the sort**, and there is no `agent.view.get`: what was sorted on is
    /// unreadable once sent.
    pub async fn set_agent_view(
        &self,
        source: &str,
        token: &str,
        order: SortOrder,
        label: &str,
    ) -> Result<AgentView> {
        self.call(
            "agent.view.set",
            serde_json::json!({
                "source": source,
                "sort": [{ "field": { "token": token }, "order": order.as_str() }],
                "label": label,
            }),
        )
        .await
    }

    /// Puts the desk's own agent order back.
    ///
    /// **This takes no source and is not scoped to one.** It clears whatever view is active,
    /// whoever set it, so a caller that never set one must not call it.
    pub async fn clear_agent_view(&self) -> Result<AgentView> {
        self.call("agent.view.clear", serde_json::json!({})).await
    }

    /// The title herdr is *showing* for a pane — whoever's report is winning the field.
    pub async fn pane_title(&self, pane_id: &str) -> Result<Option<String>> {
        let r: model::PaneReply = self
            .call("pane.get", serde_json::json!({ "pane_id": pane_id }))
            .await?;
        Ok(r.pane.title)
    }

    pub async fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        let _: serde_json::Value = self
            .call(
                "pane.send_text",
                serde_json::json!({ "pane_id": pane_id, "text": text }),
            )
            .await?;
        Ok(())
    }

    pub async fn send_keys<S: AsRef<str> + Sync>(&self, pane_id: &str, keys: &[S]) -> Result<()> {
        let keys: Vec<&str> = keys.iter().map(AsRef::as_ref).collect();
        let _: serde_json::Value = self
            .call(
                "pane.send_keys",
                serde_json::json!({ "pane_id": pane_id, "keys": keys }),
            )
            .await?;
        Ok(())
    }

    /// The visible screen, in the form the ring is built from.
    ///
    /// Its row count is what turns `scroll.max_offset_from_bottom` into the pane's last *content*
    /// row, because `recent` stops there rather than at the bottom of the grid (#519). `visible` is
    /// outside the harvest gate of #513 — that wants `recent`/`recent_unwrapped` with
    /// `format: "text"` — so this is safe at any depth.
    pub async fn read_visible(&self, pane_id: &str) -> Result<model::Read> {
        let r: model::ReadReply = self
            .call(
                "pane.read",
                serde_json::json!({
                    "pane_id": pane_id, "source": "visible", "format": "ansi"
                }),
            )
            .await?;
        Ok(r.read)
    }

    /// The pane's **exact** column count, read rather than inferred.
    ///
    /// `pane.selection.read` answers a cursor column below the grid width and refuses one at or
    /// past it with `selection_unavailable`, on any valid row and whatever the row holds — blank
    /// rows and rows of double-width glyphs bound identically (#509). So the width is the one `W`
    /// with `fits(W - 1)` and `!fits(W)`, and a binary search finds it in about twelve pure reads.
    ///
    /// This is what #221 said the API did not have. Nothing *reports* a column count even in 0.9 —
    /// the layout rect is still fiction on a headless pane (#68/#84) and the four `width` fields in
    /// the schema are still the same four decoys — but a method that *bounds* on the real one is
    /// the same answer arriving by another door, and an exact one where the inference it replaces
    /// could not separate `n` from `n + 1` on a screen of wide glyphs (#220).
    ///
    /// `hint` is what the caller already believes — the last width read, or the layout rect, which
    /// is the width or one more than it (#230). A hint that is right costs **two** calls; only a
    /// pane that has moved pays for the search. The read mutates nothing: it does not move the
    /// scroll offset and it does not clear `done` (#509).
    pub async fn pane_width(&self, pane_id: &str, hint: Option<u16>) -> Result<u16> {
        // Well past any real terminal, and the search is logarithmic in it.
        const CEILING: u16 = 4096;

        if let Some(hint) = hint.filter(|h| (1..=CEILING).contains(h))
            && self.fits(pane_id, hint - 1).await?
            && !self.fits(pane_id, hint).await?
        {
            return Ok(hint);
        }
        let (mut lo, mut hi) = (0u16, CEILING);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            match self.fits(pane_id, mid).await? {
                true => lo = mid,
                false => hi = mid - 1,
            }
        }
        // The search never asks about column 0, so a grid one column wide and a pane that answers
        // nothing at all converge to the same place. Only that case pays for the extra call, and
        // it must be paid: answering `1` for an unmeasurable pane would put its stream at one
        // column rather than leaving the caller with the width it already had — a plausible
        // wrong answer, which is the shape #233 was made of.
        if lo == 0 && !self.fits(pane_id, 0).await? {
            bail!("pane {pane_id} refused column 0, so it has no readable row to measure against");
        }
        // **Saturating at the ceiling is not a width, it is a herdr that does not bound.** The
        // whole method rests on `selection_unavailable` past the grid (#509); something that
        // answers every column with a success has no such edge, and believing it would open a
        // stream at four thousand columns. No terminal is this wide, so the number is evidence
        // rather than a measurement.
        if lo >= CEILING {
            bail!(
                "pane {pane_id} accepted every column up to {CEILING}, so this herdr does not \
                 bound a selection at the grid width and cannot be measured"
            );
        }
        Ok(lo + 1)
    }

    /// Every match for `query` in the pane's **whole** scrollback, positioned in rows from the
    /// bottom.
    ///
    /// `pane.copy_search` searches all of herdr's retained history rather than the 1000-row window
    /// `pane.read recent` caps at (#510/#511), and it is a pure read — it does not move the pane's
    /// scroll offset and it does not clear `done` (#515). It answers in herdr's absolute row space,
    /// where row 0 is the oldest row still retained; this converts to distance from the live row,
    /// which is the coordinate a client's own ring shares.
    ///
    /// It needs a live `content_revision`, and only `copy_motion` and `copy_search` return one — no
    /// pane record and no event carries it (#511) — so a cheap motion is taken first. A pane that
    /// scrolls between the two answers `stale_content`, which is a retry rather than a failure:
    /// the query is still good, the screen simply moved.
    pub async fn find(
        &self,
        pane_id: &str,
        query: &str,
        backward: bool,
        from: Option<u32>,
    ) -> Result<model::Matches> {
        let bottom = self.bottom_row(pane_id).await?;
        let revision = self.content_revision(pane_id).await?;
        let start = i64::from(bottom) - i64::from(from.unwrap_or(0));
        let reply: model::SearchReply = self
            .call(
                "pane.copy_search",
                serde_json::json!({
                    "pane_id": pane_id,
                    "query": query,
                    "direction": if backward { "backward" } else { "forward" },
                    "cursor": { "row": start.max(0), "col": 0 },
                    "content_revision": revision,
                }),
            )
            .await?;
        let hits = reply
            .matches
            .into_iter()
            .map(|m| model::Hit {
                from_bottom: bottom.saturating_sub(m.start.row),
                col: m.start.col,
                end_from_bottom: bottom.saturating_sub(m.end.row),
                end_col: m.end.col,
                row: m.start.row,
            })
            .collect();
        Ok(model::Matches {
            hits,
            total: reply.total,
            current: reply.current,
        })
    }

    /// A range of absolute scrollback rows, plain — no SGR and no OSC 8 (#510).
    ///
    /// One call, whatever the depth: 4303 rows came back in 2 ms and 44 KB, where reading a row at
    /// a time costs a fresh socket per row and 14.5 ms with it (#521). The caller re-splits.
    pub async fn rows_text(&self, pane_id: &str, from: u32, to: u32, cols: u16) -> Result<String> {
        let reply: model::SelectionReply = self
            .call(
                "pane.selection.read",
                serde_json::json!({
                    "pane_id": pane_id,
                    "anchor": { "row": from, "col": 0 },
                    "cursor": { "row": to, "col": cols.saturating_sub(1) },
                }),
            )
            .await?;
        Ok(reply.text)
    }

    /// The text of one absolute scrollback row, plain — no SGR and no OSC 8 (#510).
    pub async fn row_text(&self, pane_id: &str, row: u32, cols: u16) -> Result<String> {
        if cols == 0 {
            return Ok(String::new());
        }
        let reply: model::SelectionReply = self
            .call(
                "pane.selection.read",
                serde_json::json!({
                    "pane_id": pane_id,
                    "anchor": { "row": row, "col": 0 },
                    "cursor": { "row": row, "col": cols - 1 },
                }),
            )
            .await?;
        Ok(reply.text)
    }

    /// The index of the live row: everything above it is history, and it is what `from_bottom` is
    /// measured against.
    async fn bottom_row(&self, pane_id: &str) -> Result<u32> {
        let reply: model::PaneReply = self
            .call("pane.get", serde_json::json!({ "pane_id": pane_id }))
            .await?;
        let scroll = reply
            .pane
            .scroll
            .context("herdr reported no scroll state for this pane")?;
        let depth = scroll.max_offset_from_bottom + scroll.viewport_rows;
        Ok(u32::try_from(depth.saturating_sub(1)).unwrap_or(u32::MAX))
    }

    /// The stale-guard `copy_search` requires, taken from the one op that hands it over cheaply.
    async fn content_revision(&self, pane_id: &str) -> Result<u64> {
        let reply: model::MotionReply = self
            .call(
                "pane.copy_motion",
                serde_json::json!({
                    "pane_id": pane_id,
                    "cursor": { "row": 0, "col": 0 },
                    "motion": "line_end",
                }),
            )
            .await?;
        Ok(reply.content_revision)
    }

    /// Whether `col` is inside the pane's grid. Row 0 is always a row the pane has.
    async fn fits(&self, pane_id: &str, col: u16) -> Result<bool> {
        let asked = self
            .call::<serde_json::Value>(
                "pane.selection.read",
                serde_json::json!({
                    "pane_id": pane_id,
                    "anchor": { "row": 0, "col": 0 },
                    "cursor": { "row": 0, "col": col },
                }),
            )
            .await;
        match asked {
            Ok(_) => Ok(true),
            // The refusal *is* the measurement, so it is the one error this must not propagate.
            // Everything else — a closed pane, a dead socket — is a failed read and says nothing
            // about the width, which is the difference #233 was made of.
            Err(e) if is_code(&e, "selection_unavailable") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Reads herdr's scrollback ring. Callers must gate this on
    /// [`Pane::scrollback_is_safe_to_read`] before asking for more than the viewport.
    pub async fn read_scrollback(&self, pane_id: &str, lines: u64) -> Result<model::Read> {
        let r: model::ReadReply = self
            .call(
                "pane.read",
                serde_json::json!({
                    "pane_id": pane_id, "source": "recent", "lines": lines, "format": "ansi"
                }),
            )
            .await?;
        Ok(r.read)
    }
}

/// Whether an error from a call is herdr's own refusal with this code, rather than a transport
/// failure wearing the same `Display`.
fn is_code(e: &anyhow::Error, code: &str) -> bool {
    e.downcast_ref::<RpcError>().is_some_and(|r| r.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::sync::oneshot;

    /// Answers one `pane.read` and hands back the request line it was asked with.
    fn recording() -> (tempfile::TempDir, PathBuf, oneshot::Receiver<String>) {
        let dir = tempfile::tempdir().expect("a dir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .expect("a request");
            let reply = serde_json::json!({
                "result": { "read": { "text": "", "truncated": false } }
            });
            let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
            let _ = stream.flush().await;
            let _ = tx.send(line);
        });
        (dir, socket, rx)
    }

    /// **The scrollback read must stay on `format: "ansi"`, and this is not about colour.**
    ///
    /// On a pane whose detected agent is *idle* and showing the alternate screen with mouse
    /// reporting on, herdr answers a `pane.read` of `format: "text"` and `source: "recent"` or
    /// `"recent_unwrapped"` — with `lines` above the screen's row count — by **injecting synthetic
    /// mouse-wheel events into the agent's TUI** to harvest history, and then scrolling it back
    /// down. Measured at 5.3 s for that call against 0 ms for the same read in `ansi`, on 0.9.0
    /// and identically on 0.8.2 (probe #513, superseding #231).
    ///
    /// The operator watching that pane sees it scroll. Rule 3 forbids exactly this, and the only
    /// thing standing between this call and it is the format. Changing it back to `"text"` for
    /// any reason — cheaper parsing, a stripped-ANSI convenience — reintroduces the defect
    /// silently, on the pane of whoever is being read.
    #[tokio::test]
    async fn the_scrollback_read_never_asks_for_text_and_never_makes_herdr_scroll_a_pane() {
        let (_dir, socket, asked) = recording();
        Herdr::new(&socket)
            .read_scrollback("w1:p1", 4096)
            .await
            .expect("the read");

        let sent: serde_json::Value =
            serde_json::from_str(asked.await.expect("the request").trim()).expect("json");
        let params = &sent["params"];
        assert_eq!(
            params["format"], "ansi",
            "a `text` scrollback read makes herdr wheel-scroll an idle agent's alt screen (#513)"
        );
        assert_eq!(params["source"], "recent");
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    /// A herdr whose grid is `width` columns wide, answering `pane.selection.read` the way 0.9
    /// does: any cursor column below the width, and `selection_unavailable` at it or past it.
    fn a_grid(
        width: u16,
    ) -> (
        tempfile::TempDir,
        PathBuf,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let dir = tempfile::tempdir().expect("a dir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).await.is_err() {
                    continue;
                }
                counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let req: serde_json::Value = serde_json::from_str(&line).expect("json");
                let col = req["params"]["cursor"]["col"].as_u64().unwrap_or(0);
                let reply = match col < width as u64 {
                    true => serde_json::json!({ "result": { "text": "" } }),
                    false => serde_json::json!({
                        "error": { "code": "selection_unavailable", "message": "selection text is unavailable" }
                    }),
                };
                let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        (dir, socket, calls)
    }

    #[tokio::test]
    async fn the_width_is_read_exactly_at_every_size_and_a_right_hint_costs_two_calls() {
        for width in [1u16, 43, 80, 119, 150, 200, 512, 4096] {
            let (_dir, socket, calls) = a_grid(width);
            let herdr = Herdr::new(&socket);

            assert_eq!(
                herdr.pane_width("w1:p1", None).await.expect("a width"),
                width,
                "the search missed a {width}-column grid"
            );
            let searched = calls.swap(0, std::sync::atomic::Ordering::Relaxed);

            assert_eq!(
                herdr.pane_width("w1:p1", Some(width)).await.expect("a width"),
                width
            );
            assert_eq!(
                calls.swap(0, std::sync::atomic::Ordering::Relaxed),
                2,
                "a hint that is already right must cost one confirming pair, not a search"
            );
            assert!(
                searched <= 26,
                "the cold search took {searched} calls for {width} columns, which is not logarithmic"
            );
        }
    }

    /// The rect is the width or one more than it — the column herdr keeps back for the scrollbar
    /// (#230) — so the hint is wrong exactly half the time, and being wrong must still answer.
    #[tokio::test]
    async fn a_hint_one_column_out_still_lands_on_the_real_width() {
        let (_dir, socket, _calls) = a_grid(119);
        let herdr = Herdr::new(&socket);
        assert_eq!(herdr.pane_width("w1:p1", Some(120)).await.expect("a width"), 119);
        assert_eq!(herdr.pane_width("w1:p1", Some(1)).await.expect("a width"), 119);
    }

    /// **A herdr that answers every column is not a very wide pane.** The measurement *is* the
    /// refusal past the grid (#509); something with no such edge — a build with no
    /// `pane.selection.read`, a fake, a proxy that says `ok` to anything — has told us nothing, and
    /// a stream opened at the ceiling would be four thousand columns of padding on every row.
    #[tokio::test]
    async fn a_herdr_that_never_refuses_a_column_has_not_measured_anything() {
        let (_dir, socket, _calls) = a_grid(u16::MAX);
        let width = Herdr::new(&socket).pane_width("w1:p1", None).await;
        assert!(
            width.is_err(),
            "a bound that never fires was reported as a width of {width:?}"
        );
    }

    /// A pane that refuses column 0 is not a zero-column pane, it is a pane this cannot measure —
    /// and answering "0" would put a stream at no width at all rather than leaving the caller
    /// with what it had. #233 is what a plausible-looking answer costs here.
    #[tokio::test]
    async fn a_pane_that_answers_nothing_is_an_error_rather_than_a_width_of_zero() {
        let (_dir, socket, _calls) = a_grid(0);
        assert!(Herdr::new(&socket).pane_width("w1:p1", None).await.is_err());
    }
}
