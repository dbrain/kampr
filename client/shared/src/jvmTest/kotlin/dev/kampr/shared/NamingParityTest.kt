package dev.kampr.shared

import dev.kampr.shared.model.Naming
import dev.kampr.shared.model.Template
import dev.kampr.shared.model.TemplateException
import dev.kampr.shared.model.fieldsOf
import dev.kampr.shared.model.homeRelative
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import kotlin.test.fail

// `crates/kampr-core/tests/naming.rs` renders this exact file too, and asserts the same strings.
// Neither side owns it: a phone and a CLI that disagree about what a pane is called are two
// clients, not one.
private const val FIXTURE = "crates/kampr-core/tests/fixtures/naming-cases.json"

class NamingParityTest {
    private val cases: Map<String, JsonObject> by lazy {
        var dir = File(".").absoluteFile
        repeat(5) {
            val candidate = File(dir, FIXTURE)
            if (candidate.isFile) {
                return@lazy (Wire.json.parseToJsonElement(candidate.readText()) as JsonObject)
                    .filterValues { it is JsonObject }
                    .mapValues { (_, v) -> v as JsonObject }
            }
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not find $FIXTURE from ${File(".").absolutePath}")
    }

    private fun pane(fields: JsonObject): PaneInfo {
        fun s(key: String): String? = (fields[key] as? JsonPrimitive)?.contentOrNull
        return PaneInfo(
            id = s("pane") ?: fail("every case names its pane"),
            nodeId = "01JNODE",
            workspace = s("workspace"),
            tab = s("tab"),
            cwd = s("cwd"),
            label = s("label"),
            agent = s("agent"),
            agentStatus = s("status") ?: "unknown",
            cmd = s("cmd"),
            argv = s("argv"),
        )
    }

    // `title` is the session's own — the harness writes it, the wire does not carry it yet — so it
    // is overlaid onto the fields rather than read off a `PaneInfo`.
    private fun fields(case: JsonObject) =
        fieldsOf(pane(case), title = (case["title"] as? JsonPrimitive)?.contentOrNull)

    @Test
    fun `every shipped case renders the string the fixture pins`() {
        assertTrue(cases.size > 10, "the fixture is meant to carry the cases, and it carried ${cases.size}")
        for ((name, case) in cases) {
            val expect = case["expect"]!!.jsonPrimitive.content
            val path = (case["home_relative"] as? JsonPrimitive)?.contentOrNull
            if (path != null) {
                assertEquals(expect, homeRelative(path), name)
                continue
            }
            val template = Template.parse(case["template"]!!.jsonPrimitive.content)
            assertEquals(expect, template.render(fields(case["fields"]!!.jsonObject)), name)
        }
    }

    // The default the three clients share is a value in the fixture too, so a change to it on one
    // side cannot land without the other.
    @Test
    fun `the default template is the one the fixture pins`() {
        assertEquals(
            "{label|title|workspace|cwd|pane} · {argv|cmd|agent|'bash'}",
            Naming.DEFAULT_TEMPLATE,
        )
    }

    @Test
    fun `a pane whose command ble sh hid is named after its shell rather than after a job`() {
        val busy = PaneInfo(
            id = "01JNODE/w3:p2",
            nodeId = "01JNODE",
            workspace = "kampr",
            cwd = "/home/dbrain/dev/kampr",
            cmd = "cargo",
            argv = "cargo test",
        )
        assertEquals("kampr · cargo test", paneTitle(busy))
        assertEquals("kampr · bash", paneTitle(busy.copy(cmd = null, argv = null)))
    }

    // The operator's rule: automatic only where nothing manual exists. `label` is what they typed
    // on the pane and `title` is what the harness called the conversation, so a template that puts
    // the generated one first is the defect this names.
    @Test
    fun `a name the operator set by hand beats a title the session generated`() {
        val pane = PaneInfo(id = "01JNODE/w3:p2", nodeId = "01JNODE", workspace = "kampr", agent = "claude")
        val title = "the width inference rewrite"

        assertEquals(
            "the width inference rewrite · claude",
            Naming.default.render(fieldsOf(pane, title = title)),
        )
        assertEquals(
            "build · claude",
            Naming.default.render(fieldsOf(pane.copy(label = "build"), title = title)),
        )
        assertEquals("build · claude", Naming.default.render(fieldsOf(pane.copy(label = "build"))))
    }

    // `{last_cmd}` and `{branch}` have no source behind them (11-cli-briefs W9), so asking for one
    // is a typo to say out loud rather than a section that renders nothing for ever.
    @Test
    fun `a token with no source behind it is refused by name`() {
        assertFailsWith<TemplateException> { Template.parse("{workspace} {last_cmd}") }
        assertFailsWith<TemplateException> { Template.parse("{branch}") }
    }

    @Test
    fun `a malformed template says which way it is malformed`() {
        assertFailsWith<TemplateException> { Template.parse("{workspace") }
        assertFailsWith<TemplateException> { Template.parse("[{workspace}") }
        assertFailsWith<TemplateException> { Template.parse("{workspace}]") }
        assertFailsWith<TemplateException> { Template.parse("{}") }
        assertFailsWith<TemplateException> { Template.parse("{'oops}") }
    }
}
