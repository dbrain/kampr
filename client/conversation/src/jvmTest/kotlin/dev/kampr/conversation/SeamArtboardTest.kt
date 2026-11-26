package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import java.io.File
import kotlin.test.Test

// The pane judged on a real screen, the way the plan asks for it: the board-7 floor, a source
// that declares almost nothing, and the rich source beside it. The floor must show no empty rows,
// no disabled controls and no gaps; the rich one must show every piece the source declared.
class SeamArtboardTest {
    private val out = File("build/artboards")
    private val now = 1_770_000_000_000.0

    private fun spoken(id: String, role: String, at: String, text: String) = Turn(id, role, at, listOf(Block.Md(text)))

    private val page = """
        {"t":"convo","pane":"$PANE_ID","cursor":"c-1","more":false,"turns":[
          {"id":"u-1","role":"user","at":"2026-01-01T09:00:01Z","blocks":[{"b":"md","text":"watch this shell and open the worker"}]},
          {"id":"a-2","role":"assistant","at":"2026-01-01T09:00:04Z","blocks":[
            {"b":"md","text":"Starting the probe in the background."},
            {"b":"tool","name":"Bash","summary":"probe --bg","lines":12,"state":"running"}]}
        ]}
    """.trimIndent()

    // Board 7, the floor: the source says it has almost nothing, and the pane must take no room
    // for the pieces it does not have.
    @Test
    fun floorSourceDrawsNothingItDidNotDeclare() {
        val (_, pane) = demoPane(
            page,
            """{"t":"convo.abilities","pane":"$PANE_ID","abilities":{"input":[],"steer":[],"asks":[],"streams":[],"facts":[],"lifecycle":[]}}""",
        )
        renderArtboard(390.dp, 844.dp, SoftTheme, TypeScale.Phone, File(out, "seam-floor-390.png")) {
            val tokens = Kampr.tokens
            Modifier.fillMaxSize().background(tokens.color.bg).let {
                ConversationView(pane, demoInfo(), it, clock = { now })
            }
        }
    }

    // The rich source: everything the seam can declare, all at once. Every piece on the screen is
    // a frame this source sent, and the source's own words.
    @Test
    fun richSourceDrawsEveryPieceItDeclared() {
        val (_, pane) = demoPane(
            page,
            """{"t":"convo.abilities","pane":"$PANE_ID","abilities":{
               "input":["images","slash","mention","queue_next","send_now"],
               "steer":["interrupt","compact"],
               "asks":["permission","always","edit_input","question","plan"],
               "streams":["text","tool_args","thinking"],
               "facts":["account","billing","limits","context"],
               "lifecycle":[]}}""",
            """{"t":"convo.state","pane":"$PANE_ID","state":"running"}""",
            """{"t":"convo.live","pane":"$PANE_ID","live":{"text":"Running the probe, then I will read the output","thinking":412,
               "tools":[{"id":"t1","tool":"Bash","args":"{\"command\": \"probe --bg\"}","state":"running"}]}}""",
            """{"t":"convo.tasks","pane":"$PANE_ID","tasks":[
               {"id":"w1","kind":"agent","title":"probe worker","call":"probe --bg","since":"2026-01-01T08:58:40Z","state":"running"},
               {"id":"s1","kind":"shell","call":"tail -f probe.log","since":"2026-01-01T08:59:10Z","state":"running"}]}""",
            """{"t":"convo.trouble","pane":"$PANE_ID","trouble":{
               "kind":"api","text":"529 overloaded, retrying","status":"retrying",
               "retry":{"attempt":2,"of":5,"in_ms":12000},"fix":"billing"}}""",
            """{"t":"convo.session","pane":"$PANE_ID","facts":{
               "id":"s-1","cwd":"/home/dbrain/dev/kampr","title":"probe","model":"sonnet","mode":"plan","version":"2.1.239",
               "account":"max","billing":{"provider":"anthropic","key":"max","rate_limited":false},"started":"2026-01-01T08:30:00Z"}}""",
            """{"t":"convo.controls","pane":"$PANE_ID","controls":[
               {"id":"model","label":"Model","kind":"choice","value":"sonnet",
                "options":[{"value":"sonnet","label":"Sonnet"},{"value":"opus","label":"Opus","detail":"deeper"}]},
               {"id":"fast","label":"Fast mode","kind":"toggle","value":false,"confirm":"Billed outside your plan \u00b7 asks first"}]}""",
            """{"t":"convo.actions","pane":"$PANE_ID","actions":[
               {"id":"compact","label":"Compact","detail":"Summarise the conversation"},
               {"id":"rewind","label":"Rewind","detail":"Undo the last turn"}]}""",
            """{"t":"convo.limits","pane":"$PANE_ID","limits":{"plan":"Max","usage":{"five_hour":0.46},"resets_at":"2026-01-01T17:00:00Z"}}""",
            """{"t":"convo.context","pane":"$PANE_ID","context":{"used":120000,"total":200000}}""",
        )
        renderArtboard(1440.dp, 900.dp, SoftTheme, TypeScale.Desk, File(out, "seam-rich-1440.png")) {
            val tokens = Kampr.tokens
            Modifier.fillMaxSize().background(tokens.color.bg).let {
                ConversationView(pane, demoInfo(), it, clock = { now })
            }
        }
    }
}
