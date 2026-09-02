package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.AttachmentBytes
import androidx.compose.runtime.CompositionLocalProvider
import dev.kampr.shared.ui.LocalPaneIo
import kotlinx.coroutines.runBlocking
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.PhosphorTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val OUT = File("build/artboards")

private fun spoken(id: String, role: String, at: String, text: String) =
    Turn(id, role, at, listOf(Block.Md(text)))

class ArtboardTest {
    private fun mobile(landscape: Boolean): @Composable () -> Unit = {
        val (_, pane) = demoPane(RICH_CONVO)
        PaneScreenMobile(
            pane = pane,
            info = demoInfo(),
            view = PaneView.Conversation,
            surfaces = ConversationSurfaces(),
            landscape = landscape,
            readOnly = false,
            onBack = {},
            onView = {},
        )
    }

    @Test
    fun portraitArtboardRenders() {
        val image = renderArtboard(
            PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
            File(OUT, "conversation-portrait.png"), content = mobile(landscape = false),
        )
        assertTrue(image.width == 780 && image.height == 1688, "${image.width}x${image.height}")
    }

    // Both speakers on one screen, which is the only way to look at what tells them apart. The
    // reply is long enough to wrap so its card reaches the same right edge the answer's does.
    @Test
    fun bothSpeakersArtboardRenders() {
        renderArtboard(PORTRAIT.first, 780.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-speakers.png")) {
            val store = KamprStore()
            store.accept(
                ServerMsg.Convo(
                    pane = PANE_ID, cursor = "u-1", more = false,
                    turns = listOf(
                        spoken(
                            "u-1", "user", "2026-08-20T09:00:01Z",
                            "which keys does the grammar accept, and does the review strip already draw them?",
                        ),
                        spoken(
                            "a-2", "assistant", "2026-08-20T09:00:22Z",
                            "Six.\n\nThey are in the probe log beside the measurement that established each one.",
                        ),
                        Turn(
                            "a-3", "assistant", "2026-08-20T09:02:41Z",
                            listOf(Block.Tool("Read", "docs/03-probe-log.md", 74, "done")),
                        ),
                        spoken("a-4", "assistant", "2026-08-20T09:04:22Z", "The strip draws four of them."),
                        spoken("u-3", "user", "2026-08-20T09:05:10Z", "thanks"),
                    ),
                ),
            )
            ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
        }
    }

    // The line that replaced a yellow badge in the header: what the agent is doing, named off the
    // transcript, and how long this reply has been going.
    @Test
    fun aWorkingReplyArtboardRenders() {
        renderArtboard(PORTRAIT.first, 420.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-working.png")) {
            val store = KamprStore()
            store.accept(
                ServerMsg.Convo(
                    pane = PANE_ID, cursor = "u-1", more = false,
                    turns = listOf(
                        spoken("u-1", "user", "2026-08-20T09:00:01Z", "run the whole gate please"),
                        spoken("a-2", "assistant", "2026-08-20T09:00:04Z", "Running it now."),
                        Turn(
                            "a-3", "assistant", "2026-08-20T09:00:09Z",
                            listOf(Block.Tool("Bash", "cargo test --workspace", null, "running")),
                        ),
                    ),
                ),
            )
            // Fixed, or the stopwatch in the picture reads a different number every render.
            val at = requireNotNull(parseIsoMillis("2026-08-20T09:02:20Z"))
            ConversationView(
                store.pane(PANE_ID), demoInfo(status = "working"), Modifier.fillMaxSize(),
            ) { at }
        }
    }

    // A reply nobody has opened: one box with a top and a bottom and no middle, keeping its clock
    // and the line it shows of itself so the reader can tell what they put away.
    @Test
    fun aPutAwayReplyArtboardRenders() {
        renderArtboard(PORTRAIT.first, 220.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-away.png")) {
            val reply = Reply(
                listOf(
                    spoken("a-2", "assistant", "2026-08-20T09:00:04Z", "Running it now."),
                    Turn(
                        "a-3", "assistant", "2026-08-20T09:04:09Z",
                        listOf(Block.Tool("Bash", "cargo test --workspace", 812, "done")),
                    ),
                    spoken("a-4", "assistant", "2026-08-20T09:06:20Z", "All green."),
                ),
            )
            val at = requireNotNull(parseIsoMillis("2026-08-20T09:06:30Z"))
            Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(0.dp)) {
                    ReplyHead(reply, "claude", at, collapsed = true, onToggle = {}, edge = BlockEdge.Only)
                    ReplyHead(reply, "claude", at, collapsed = false, onToggle = {}, edge = BlockEdge.Head)
                }
            }
        }
    }

    // What the agent shared, as something a reader can decide about: a thumbnail, its name and its
    // size, above the button it looked like before the bytes arrived. The picture itself is a tap
    // away, where it has the screen to itself — a 900x1400 screenshot of a wide pane is unreadable
    // at any size this column can give it.
    @Test
    fun aSharedFileArtboardRenders() {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")))
        val fetched = AttachmentStore(PANE_ID)
        runBlocking { fetched.open(node, SHOT) }
        renderArtboard(PORTRAIT.first, 300.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-shared.png")) {
            CompositionLocalProvider(LocalPaneIo provides node) {
                Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        AttachmentCard(SHOT, fetched)
                        AttachmentCard(SHOT, AttachmentStore(PANE_ID))
                    }
                }
            }
        }
    }

    // The picture open on a phone, inside the pane screen it is opened from — which is where the
    // question "how do I get out of this" is answered, and the artboard is how that answer is
    // checked rather than assumed.
    @Test
    fun anOpenPictureArtboardRenders() {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")))
        val fetched = AttachmentStore(PANE_ID)
        runBlocking { fetched.open(node, SHOT) }
        fetched.view(SHOT)
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-open.png")) {
            val shown = fetched.state(SHOT.id) as AttachmentState.Shown
            PaneScreenMobile(
                pane = runPane(),
                info = demoInfo(),
                view = PaneView.Conversation,
                surfaces = object : dev.kampr.shared.ui.PaneSurfaces by ConversationSurfaces() {
                    @Composable
                    override fun Conversation(
                        pane: dev.kampr.shared.model.PaneState,
                        info: dev.kampr.shared.wire.PaneInfo?,
                        modifier: Modifier,
                    ) = ImageViewer(
                        shown.image, headlineOf(SHOT), detailOf(SHOT), {}, {}, null, modifier,
                    )
                },
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
            )
        }
    }

    @Test
    fun landscapeArtboardRenders() {
        renderArtboard(
            LANDSCAPE.first, LANDSCAPE.second, SoftTheme, TypeScale.Phone,
            File(OUT, "conversation-landscape.png"), content = mobile(landscape = true),
        )
    }

    @Test
    fun desktopArtboardRenders() {
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, File(OUT, "conversation-desktop.png")) {
            val (_, pane) = demoPane(RICH_CONVO)
            PaneScreenDesktop(
                pane = pane,
                info = demoInfo(),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                readOnly = false,
                onView = {},
            )
        }
    }

    // A second theme is the only thing that catches a colour that never went through the tokens.
    @Test
    fun portraitRendersInASecondTheme() {
        renderArtboard(
            PORTRAIT.first, PORTRAIT.second, PhosphorTheme, TypeScale.Phone,
            File(OUT, "conversation-portrait-phosphor.png"), content = mobile(landscape = false),
        )
    }

    @Test
    fun codexTranscriptRenders() {
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-codex.png")) {
            val (_, pane) = demoPane(CODEX_CONVO.replace("01JNODE.../w4:p1", PANE_ID))
            PaneScreenMobile(
                pane = pane,
                info = demoInfo(agent = "codex"),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
            )
        }
    }

    // Visual evidence of a turn mid-flight: the preview and the mark that says it is one. The
    // assertion that the mark is *there* is a semantics one, in AccessibilityTest.
    @Test
    fun aLiveTurnRenders() {
        renderArtboard(PORTRAIT.first, 300.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-live.png")) {
            val (_, pane) = demoPane(RICH_CONVO, LIVE_TURN)
            Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                TurnView(
                    turn = pane.turns.first { it.id == LIVE_TURN_ID },
                    query = "",
                    expanded = emptyList(),
                    onToggle = {},
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }

    // The report's own screen: a wall of tool cards with a sentence of prose in the middle, which
    // is what makes it two runs. Collapsed, because collapsed is what the reader is handed.
    @Test
    fun aRunOfToolCallsRenders() {
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-tool-run.png")) {
            PaneScreenMobile(
                pane = runPane(),
                info = demoInfo(),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
            )
        }
    }

    // Both levels at once: the run opened onto its cards, and the first of those opened onto its
    // own output.
    @Test
    fun anOpenedRunOfToolCallsRenders() {
        renderArtboard(PORTRAIT.first, 460.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-tool-run-open.png")) {
            val row = transcriptRows(TOOL_RUN_TURNS, "").filterIsInstance<TranscriptRow.Run>().first()
            Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                ToolRunCard(row, expanded = true, onToggle = {}) {
                    for (turn in row.turns) {
                        // `framed = false`, because that is what the transcript passes: the run's
                        // own card is the frame, and an artboard that draws a second one inside it
                        // is a picture of a screen nobody has.
                        TurnView(
                            turn, "", listOf("${row.turns.first().id}#0"), {},
                            Modifier.fillMaxWidth(), framed = false,
                        )
                    }
                }
            }
        }
    }

    // A message put away, beside one that is not: the folded row keeps its age and its first line
    // so the reader can tell what they folded. The clock is fixed rather than read, or the artboard
    // ages by a day every day.
    @Test
    fun aFoldedMessageRenders() {
        renderArtboard(PORTRAIT.first, 430.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-folded.png")) {
            val at = "2026-08-23T09:00:00.000Z"
            val now = requireNotNull(parseIsoMillis(at)) + 12 * 60_000
            val ask = Turn("u-1", "user", at, listOf(Block.Md("what did the width inference land on?")))
            val put = Turn(
                "a-2", "assistant", at,
                listOf(
                    Block.Md(
                        "## Where the 74 comes from\n\nThe layout rect is not the PTY: it is the pane's " +
                            "outer box, and the column it keeps back is the scrollbar's.\n\nSo the width " +
                            "has to be inferred.",
                    ),
                ),
            )
            val open = Turn(
                "a-3", "assistant", at,
                listOf(Block.Md("Two panes agreed on 74, and the third\n\ndisagreed until the margin was paid.")),
            )
            Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    TurnView(ask, "", emptyList(), {}, Modifier.fillMaxWidth(), now = now)
                    TurnView(put, "", listOf("fold:a-2"), {}, Modifier.fillMaxWidth(), now = now)
                    TurnView(open, "", emptyList(), {}, Modifier.fillMaxWidth(), now = now)
                }
            }
        }
    }

    @Test
    fun absentConversationRenders() {
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-absent.png")) {
            val (_, pane) = demoPane()
            PaneScreenMobile(
                pane = pane,
                info = demoInfo(agent = "aider", conversation = false),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
            )
        }
    }
}
