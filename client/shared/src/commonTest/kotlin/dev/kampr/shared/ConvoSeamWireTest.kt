package dev.kampr.shared

import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.Action
import dev.kampr.shared.wire.ArgSpec
import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ContextUse
import dev.kampr.shared.wire.Control
import dev.kampr.shared.wire.ControlOption
import dev.kampr.shared.wire.Limits
import dev.kampr.shared.wire.LiveTool
import dev.kampr.shared.wire.LiveTurn
import dev.kampr.shared.wire.PendingField
import dev.kampr.shared.wire.Reply
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionFacts
import dev.kampr.shared.wire.Suggestion
import dev.kampr.shared.wire.Task
import dev.kampr.shared.wire.TaskEnd
import dev.kampr.shared.wire.Trouble
import dev.kampr.shared.wire.TroubleFix
import dev.kampr.shared.wire.TroubleRetry
import dev.kampr.shared.wire.Wire
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The Phase 2 seam frames, checked the way the wire rule checks every field: the client's
// encoding is what the node's serde will decode, and the node's encoding is what the client must
// draw. One side's guess about the other's spelling is a pane that draws nothing.
class ConvoSeamWireTest {
    // --- Client to node: the five verbs, spelled the way the node's serde reads them.

    @Test
    fun theFiveVerbsEncodeToTheShapeTheNodeDecodes() {
        val steer = Wire.encode(ClientMsg.ConvoSteer("p", "interrupt", "hold"))
        val steerObj = Wire.json.parseToJsonElement(steer).jsonObject
        assertEquals("convo.steer", steerObj["t"]!!.jsonPrimitive.content)
        assertEquals("p", steerObj["pane"]!!.jsonPrimitive.content)
        assertEquals("interrupt", steerObj["steer"]!!.jsonPrimitive.content)
        assertEquals("hold", steerObj["text"]!!.jsonPrimitive.content)
        assertNull(Wire.json.parseToJsonElement(Wire.encode(ClientMsg.ConvoSteer("p", "stop"))).jsonObject["text"], "no text is omitted, not null")

        val set = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.ConvoSet("p", "model", JsonPrimitive("sonnet")))).jsonObject
        assertEquals("convo.set", set["t"]!!.jsonPrimitive.content)
        assertEquals("model", set["control"]!!.jsonPrimitive.content)
        assertEquals("sonnet", set["value"]!!.jsonPrimitive.content)

        val act = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.ConvoAct("p", "add", JsonObject(mapOf("name" to JsonPrimitive("repo")))))).jsonObject
        assertEquals("convo.act", act["t"]!!.jsonPrimitive.content)
        assertEquals("add", act["action"]!!.jsonPrimitive.content)
        assertEquals("repo", act["args"]!!.jsonObject["name"]!!.jsonPrimitive.content)

        val follow = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.ConvoFollow("p", "t1", 41))).jsonObject
        assertEquals("convo.follow", follow["t"]!!.jsonPrimitive.content)
        assertEquals("t1", follow["id"]!!.jsonPrimitive.content)
        assertEquals(41L, follow["from"]!!.jsonPrimitive.content.toLong())

        val suggest = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.ConvoSuggest("p", "commands", "co"))).jsonObject
        assertEquals("convo.suggest", suggest["t"]!!.jsonPrimitive.content)
        assertEquals("commands", suggest["kind"]!!.jsonPrimitive.content)
        assertEquals("co", suggest["query"]!!.jsonPrimitive.content)
    }

    // The widened answer: the key is what every node decodes, and the reply is what the seam's
    // nodes decode beside it. Absent is the old frame to the last byte.
    @Test
    fun anAnswerCarriesItsReplyAndAnOldOneCarriesNone() {
        val bare = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "1", "a1"))).jsonObject
        assertNull(bare["reply"], "an answer with no reply is the frame the node decoded before")

        val allow = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "1", "a1", reply = Reply.Allow(remember = true)))).jsonObject
        val allowObj = allow["reply"]!!.jsonObject
        assertEquals("allow", allowObj.keys.single())
        assertTrue(allowObj["allow"]!!.jsonObject["remember"]!!.jsonPrimitive.content == "true")

        val choose = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "2", "a1", reply = Reply.Choose(listOf("a", "c"))))).jsonObject
        val answers = choose["reply"]!!.jsonObject["choose"]!!.jsonObject["answers"]!!.let { it as JsonArray }
        assertEquals(listOf("a", "c"), answers.map { it.jsonPrimitive.content })

        val plan = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "3", "a1", reply = Reply.Plan(approve = false, feedback = "slower")))).jsonObject
        val planObj = plan["reply"]!!.jsonObject["plan"]!!.jsonObject
        assertEquals("false", planObj["approve"]!!.jsonPrimitive.content)
        assertEquals("slower", planObj["feedback"]!!.jsonPrimitive.content)
        assertNull(planObj["mode"], "a mode not given is absent, not null")

        val form = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "4", "a1", reply = Reply.Form(JsonObject(mapOf("name" to JsonPrimitive("x"))))))).jsonObject
        assertEquals("x", form["reply"]!!.jsonObject["form"]!!.jsonObject["values"]!!.jsonObject["name"]!!.jsonPrimitive.content)

        // serde spells a unit variant as the bare word, and the node decodes only that spelling.
        val dismiss = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Answer("p", "5", "a1", reply = Reply.Dismiss))).jsonObject
        assertEquals("dismiss", dismiss["reply"]!!.jsonPrimitive.content)
    }

    // --- Node to client: the thirteen frames, drawn from exactly what the node sends.

    @Test
    fun abilitiesAreTheWholeOfWhatThePaneMayDraw() {
        val msg = Wire.decode(
            """{"t":"convo.abilities","pane":"p","abilities":{
               "input":["images","slash","queue_next","send_now"],
               "steer":["interrupt","compact"],
               "asks":["permission","always","question"],
               "streams":["text","tool_args"],
               "facts":["account","billing","limits","context"],
               "lifecycle":[]}}""",
        ) as ServerMsg.ConvoAbilities
        assertEquals("p", msg.pane)
        assertEquals(listOf("images", "slash", "queue_next", "send_now"), msg.abilities.input)
        assertEquals(listOf("interrupt", "compact"), msg.abilities.steer)
        assertEquals(3, msg.abilities.asks.size)
        assertEquals(2, msg.abilities.streams.size)
        assertEquals(4, msg.abilities.facts.size)
        assertTrue(msg.abilities.lifecycle.isEmpty())
    }

    @Test
    fun aLiveTurnCarriesItsToolsAsTheirArgumentsArrive() {
        val msg = Wire.decode(
            """{"t":"convo.live","pane":"p","live":{
               "text":"running the probe","thinking":1200,
               "tools":[{"id":"t1","tool":"Bash","args":"{\"co","state":"running"}]}}""",
        ) as ServerMsg.ConvoLive
        assertEquals("running the probe", msg.live.text)
        assertEquals(1200L, msg.live.thinking)
        assertEquals(LiveTool("t1", "Bash", """{"co""", "running"), msg.live.tools.single())
        val withdrawn = Wire.decode("""{"t":"convo.live","pane":"p","live":{"text":"","tools":[]}}""") as ServerMsg.ConvoLive
        assertEquals(LiveTurn(), withdrawn.live)
    }

    @Test
    fun aTaskListIsTheWholeOfWhatIsRunning() {
        val msg = Wire.decode(
            """{"t":"convo.tasks","pane":"p","tasks":[
               {"id":"a1","kind":"agent","title":"worker","call":"probe","since":"2026-01-01T00:00:00Z","state":"running"},
               {"id":"s1","kind":"shell","parent":"a1","backgrounded":true,"state":"running"}]}""",
        ) as ServerMsg.ConvoTasks
        assertEquals(2, msg.tasks.size)
        assertEquals(Task("a1", "agent", "probe", null, "worker", "2026-01-01T00:00:00Z", false, "running"), msg.tasks[0])
        assertTrue(msg.tasks[1].backgrounded)
        assertEquals("a1", msg.tasks[1].parent)
    }

    @Test
    fun taskOutputAccumulatesAndEndsOnce() {
        val mid = Wire.decode(
            """{"t":"convo.task","pane":"p","id":"s1","at":12,"text":"line one\n"}""",
        ) as ServerMsg.ConvoTask
        assertEquals(12L, mid.at)
        assertEquals("line one\n", mid.text)
        assertNull(mid.ended)
        val done = Wire.decode(
            """{"t":"convo.task","pane":"p","id":"s1","at":13,"text":"","ended":{"ok":true,"summary":"done in 3s"}}""",
        ) as ServerMsg.ConvoTask
        assertEquals(TaskEnd(true, "done in 3s"), done.ended)
    }

    @Test
    fun runStateIsAnOpenWord() {
        val msg = Wire.decode("""{"t":"convo.state","pane":"p","state":"running"}""") as ServerMsg.ConvoState
        assertEquals("running", msg.state)
    }

    @Test
    fun aTroubleCarriesItsRetryAndItsFixAndAClearIsNull() {
        val retrying = Wire.decode(
            """{"t":"convo.trouble","pane":"p","trouble":{
               "kind":"api","text":"529","status":"retrying",
               "retry":{"attempt":2,"of":5,"in_ms":12000},
               "fix":{"wait":{"until":"2026-01-01T00:05:00Z"}}}}""",
        ) as ServerMsg.ConvoTrouble
        val trouble = retrying.trouble!!
        assertEquals(TroubleRetry(2, 5, 12_000L), trouble.retry)
        assertEquals(TroubleFix.Wait("2026-01-01T00:05:00Z"), trouble.fix)

        // serde spells a unit fix as the bare word, and only that spelling.
        val login = Wire.decode(
            """{"t":"convo.trouble","pane":"p","trouble":{"kind":"auth","text":"no login","status":"held","fix":"login"}}""",
        ) as ServerMsg.ConvoTrouble
        assertEquals(TroubleFix.Login, login.trouble!!.fix)
        val billing = Wire.decode(
            """{"t":"convo.trouble","pane":"p","trouble":{"kind":"billing","text":"over the limit","status":"held","fix":"billing"}}""",
        ) as ServerMsg.ConvoTrouble
        assertEquals(TroubleFix.Billing, billing.trouble!!.fix)

        val cleared = Wire.decode("""{"t":"convo.trouble","pane":"p","trouble":null}""") as ServerMsg.ConvoTrouble
        assertNull(cleared.trouble, "null is how a trouble clears")
    }

    @Test
    fun aSessionNamesWhoPays() {
        val msg = Wire.decode(
            """{"t":"convo.session","pane":"p","facts":{
               "id":"s1","cwd":"/work","title":"probe","model":"sonnet","mode":"plan","version":"0.1.0",
               "account":"max","billing":{"provider":"anthropic","key":"main","rate_limited":true},"started":"2026-01-01T00:00:00Z"}}""",
        ) as ServerMsg.ConvoSession
        assertEquals("s1", msg.facts.id)
        assertEquals(Billing("anthropic", "main", true), msg.facts.billing)
        assertEquals(SessionFacts("s1", "/work", "probe", "sonnet", "plan", "0.1.0", "max", Billing("anthropic", "main", true), "2026-01-01T00:00:00Z"), msg.facts)
    }

    @Test
    fun controlsCarryTheirValueTheirOptionsAndTheirConfirm() {
        val msg = Wire.decode(
            """{"t":"convo.controls","pane":"p","controls":[
               {"id":"model","label":"Model","kind":"choice","value":"sonnet",
                "options":[{"value":"sonnet","label":"Sonnet"},{"value":"opus","label":"Opus","detail":"deeper"}]},
               {"id":"fast","label":"Fast mode","kind":"toggle","value":false,
                "group":"speed","confirm":"Billed outside your plan \u00b7 asks first"}]}""",
        ) as ServerMsg.ConvoControls
        assertEquals(2, msg.controls.size)
        val model = msg.controls[0]
        assertEquals("choice", model.kind)
        assertEquals("sonnet", model.value!!.jsonPrimitive.content)
        assertEquals(ControlOption("opus", "Opus", "deeper"), model.options[1])
        assertNull(model.confirm)
        val fast = msg.controls[1]
        assertEquals("speed", fast.group)
        assertTrue(fast.confirm!!.isNotBlank())
    }

    @Test
    fun actionsCarryTheirDescribedArguments() {
        val msg = Wire.decode(
            """{"t":"convo.actions","pane":"p","actions":[
               {"id":"compact","label":"Compact","detail":"Summarise the conversation"},
               {"id":"add","label":"Add","args":[{"name":"name","label":"Name","required":true},
                {"name":"kind","label":"Kind","required":false,"options":["file","dir"]}]}]}""",
        ) as ServerMsg.ConvoActions
        assertEquals(2, msg.actions.size)
        assertTrue(msg.actions[0].args.isEmpty())
        assertEquals(ArgSpec("kind", "Kind", false, listOf("file", "dir")), msg.actions[1].args[1])
    }

    @Test
    fun limitsKeepTheProvidersOwnUnits() {
        val msg = Wire.decode(
            """{"t":"convo.limits","pane":"p","limits":{"plan":"Max","usage":{"five_hour":0.46},"resets_at":"2026-01-01T17:00:00Z"}}""",
        ) as ServerMsg.ConvoLimits
        assertEquals("Max", msg.limits.plan)
        assertEquals(0.46, msg.limits.usage!!.jsonObject["five_hour"]!!.jsonPrimitive.content.toDouble())
        assertEquals("2026-01-01T17:00:00Z", msg.limits.resetsAt)
        // A provider that counts in words rather than objects is drawn as-is.
        val words = Wire.decode("""{"t":"convo.limits","pane":"p","limits":{"usage":"46%"}}""") as ServerMsg.ConvoLimits
        assertEquals("46%", words.limits.usage!!.jsonPrimitive.content)
    }

    @Test
    fun contextIsTwoNumbers() {
        val msg = Wire.decode("""{"t":"convo.context","pane":"p","context":{"used":120000,"total":200000}}""") as ServerMsg.ConvoContext
        assertEquals(ContextUse(120_000L, 200_000L), msg.context)
    }

    @Test
    fun suggestionsAreKindedAndTiedToTheQuery() {
        val msg = Wire.decode(
            """{"t":"convo.suggested","pane":"p","kind":"commands","query":"co",
               "items":[{"id":"/compact","label":"Compact"},{"id":"/copy","label":"Copy","detail":"to the clipboard"}]}""",
        ) as ServerMsg.ConvoSuggested
        assertEquals("commands", msg.kind)
        assertEquals("co", msg.query)
        assertEquals(Suggestion("/copy", "Copy", "to the clipboard"), msg.items[1])
    }

    @Test
    fun anActedFrameNamesTheVerbAndWhatItSaid() {
        val ok = Wire.decode("""{"t":"convo.acted","pane":"p","action":"compact","ok":true,"text":"compacted"}""") as ServerMsg.ConvoActed
        assertTrue(ok.ok)
        assertEquals("compacted", ok.text)
        val silent = Wire.decode("""{"t":"convo.acted","pane":"p","action":"compact","ok":false}""") as ServerMsg.ConvoActed
        assertFalse(silent.ok)
        assertNull(silent.text)
    }

    // The additive rule from the node's side: a frame this build does not know is ignored, the
    // way a frame an old client does not know was always ignored.
    @Test
    fun aFrameThisBuildDoesNotKnowDecodesToNull() {
        assertNull(Wire.decode("""{"t":"convo.quantum","pane":"p"}"""))
        // And an ability word this build has never seen is data, not an error: a source grows a
        // word and the pane draws it rather than dropping the frame.
        val grew = Wire.decode("""{"t":"convo.abilities","pane":"p","abilities":{"input":["a_new_word"]}}""")
        assertEquals(ServerMsg.ConvoAbilities("p", Abilities(input = listOf("a_new_word"))), grew)
    }

    // A form's fields ride on `pending` exactly as the node's serde writes them, `type` and all.
    @Test
    fun aPendingFormDecodesItsKindAndFields() {
        val msg = Wire.decode(
            """{"t":"pending","pane":"p","question":"Pick","source":"transcript","id":"f1","kind":"form",""" +
                """"options":[{"key":"1","label":"Submit"}],""" +
                """"fields":[{"name":"colour","title":"Colour","type":"string","required":true,"choices":["red","teal"]},""" +
                """{"name":"count","detail":"how many","type":"integer"}]}""",
        ) as ServerMsg.Pending
        assertEquals("form", msg.kind)
        assertEquals(
            listOf(
                PendingField("colour", "Colour", null, "string", true, listOf("red", "teal")),
                PendingField("count", null, "how many", "integer"),
            ),
            msg.fields,
        )
        assertTrue(msg.form)
        val bare = Wire.decode("""{"t":"pending","pane":"p","question":"Q","source":"screen","options":[]}""") as ServerMsg.Pending
        assertNull(bare.kind)
        assertTrue(bare.fields.isEmpty())
    }

    // An `input` the node can queue under the client's own id, and the frame that takes it back.
    @Test
    fun aQueuedMessageCarriesItsIdAndCanBeTakenBack() {
        val queued = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.InputText("p", "run the gate", id = "m1", queue = "next"))).jsonObject
        assertEquals("input", queued["t"]!!.jsonPrimitive.content)
        assertEquals("m1", queued["id"]!!.jsonPrimitive.content)
        assertEquals("next", queued["when"]!!.jsonPrimitive.content)
        val plain = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.InputText("p", "x"))).jsonObject
        assertFalse("id" in plain || "when" in plain, "a reply to a pane says neither: $plain")
        val back = Wire.json.parseToJsonElement(Wire.encode(ClientMsg.InputUnqueue("p", "m1"))).jsonObject
        assertEquals(mapOf("t" to "input.unqueue", "pane" to "p", "id" to "m1"), back.mapValues { it.value.jsonPrimitive.content })
    }
}
