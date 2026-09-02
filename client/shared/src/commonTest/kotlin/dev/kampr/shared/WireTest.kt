package dev.kampr.shared

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class WireTest {
    @Test
    fun unknownMessageTypeIsIgnored() {
        assertNull(Wire.decode("""{"t":"teleport","pane":"x"}"""))
        assertNull(Wire.decode("""{"no_tag":1}"""))
        assertNull(Wire.decode("not json"))
    }

    @Test
    fun unknownFieldsSurviveOnKnownMessages() {
        val msg = Wire.decode(
            """{"t":"hello","protocol":1,"node_id":"01J","node_name":"cc","build":"0.1.0",
               "role":"full","caps":{"push":true,"scrollback":true,"conversation":true,"teleport":true},
               "future":{"nested":[1,2,3]}}"""
        )
        val hello = msg as ServerMsg.Hello
        assertEquals("cc", hello.nodeName)
        assertTrue(hello.caps.push)
    }

    @Test
    fun colourSpecsDecodeAllThreeShapes() {
        val msg = Wire.decode(
            """{"t":"styles","from":12,"styles":[
               {"fg":{"k":"r","v":[255,120,0]},"bg":{"k":"d"},"bold":true,"underline":true},
               {"fg":{"k":"i","v":214}},
               {"fg":{"k":"weird"}}]}"""
        ) as ServerMsg.Styles
        assertEquals(12, msg.from)
        assertEquals(ColorSpec.Rgb(255, 120, 0), msg.styles[0].fg)
        assertEquals(ColorSpec.Default, msg.styles[0].bg)
        assertTrue(msg.styles[0].bold)
        assertTrue(msg.styles[0].underline)
        assertEquals(false, msg.styles[0].italic)
        assertEquals(ColorSpec.Indexed(214), msg.styles[1].fg)
        assertEquals(ColorSpec.Default, msg.styles[2].fg)
    }

    @Test
    fun gridResetCarriesRunsCursorAndLinks() {
        val msg = Wire.decode(
            """{"t":"grid.reset","pane":"01J/w3:p2","cols":74,"rows":30,
               "rows_data":[{"row":9,"runs":[{"s":0,"x":"> 1. "},{"s":4,"x":"Yes","l":0}]}],
               "cursor":{"col":37,"row":9,"visible":true},
               "links":["https://herdr.dev"]}"""
        ) as ServerMsg.GridReset
        assertEquals(74, msg.cols)
        assertEquals(9, msg.rowsData[0].row)
        assertEquals(0, msg.rowsData[0].runs[1].l)
        assertEquals(37, msg.cursor.col)
        assertEquals(listOf("https://herdr.dev"), msg.links)
    }

    @Test
    fun gridPatchLinksAreADelta() {
        val msg = Wire.decode(
            """{"t":"grid.patch","pane":"p","rows":[{"row":9,"runs":[{"s":0,"x":"hi"}]}],
               "links":["https://example.test"]}"""
        ) as ServerMsg.GridPatch
        assertNull(msg.cursor)
        assertEquals(listOf("https://example.test"), msg.links)
    }

    @Test
    fun scrollbackRowIndexExceedsSixteenBits() {
        val msg = Wire.decode(
            """{"t":"scrollback","pane":"p","from_top":70000,
               "rows":[{"row":70001,"runs":[{"s":0,"x":"deep"}]}],
               "total_rows":90000,"complete":false,"capped":true}"""
        ) as ServerMsg.Scrollback
        assertEquals(70_001, msg.rows[0].row)
        assertTrue(msg.capped)
    }

    // Additive: a page with no `fresh` is the page every node before this one sent, and it merges.
    @Test
    fun aPageSaysWhetherItReplacesTheConversationOrMergesIntoIt() {
        val merging = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"turns":[]}"""
        ) as ServerMsg.Convo
        assertFalse(merging.fresh)
        val replacing = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"fresh":true,"turns":[]}"""
        ) as ServerMsg.Convo
        assertTrue(replacing.fresh)
    }

    @Test
    fun unknownBlockKindBecomesUnknownRatherThanFailingTheTurn() {
        val msg = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":true,"turns":[
               {"id":"t1","role":"assistant","at":"2026-08-20T13:41:55Z","blocks":[
                 {"b":"md","text":"hi"},
                 {"b":"hologram","text":"from the future"},
                 {"b":"tool","name":"Bash","summary":"probe","lines":48,"state":"done"}]}]}"""
        ) as ServerMsg.Convo
        val blocks = msg.turns[0].blocks
        assertEquals(3, blocks.size)
        assertTrue(blocks[1] is Block.Unknown)
        assertEquals("Bash", (blocks[2] as Block.Tool).name)
    }

    // A tool card's `lines` has always counted the *result*, and the only block beside it was the
    // call's own input — so a Bash card read "13 lines" over a one-line command and expanding it
    // showed the command. `role` names which side of the call a code block sits on, and it is the
    // only thing that tells the two apart on the wire.
    @Test
    fun aCodeBlockSaysWhichSideOfTheCallItIsOn() {
        val msg = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"turns":[
               {"id":"t1","role":"assistant","blocks":[
                 {"b":"tool","name":"Bash","summary":"probe key grammar","lines":3,"state":"done"},
                 {"b":"code","lang":"bash","text":"herdr key list"},
                 {"b":"code","text":"Home\nEnd\nPageUp","role":"output"}]}]}"""
        ) as ServerMsg.Convo
        val blocks = msg.turns[0].blocks
        assertNull((blocks[1] as Block.Code).role, "the call's own input is not its output")
        assertEquals("output", (blocks[2] as Block.Code).role)
        assertNull((blocks[2] as Block.Code).lang, "a result is not written in a language")
    }

    // Additive in the direction that matters: the field is absent on every code block an installed
    // client has ever been sent, and a node too old to send it leaves the reading it always gave.
    @Test
    fun aCodeBlockWithNoRoleIsTheOneEveryOlderNodeSends() {
        val msg = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"turns":[
               {"id":"t1","role":"assistant","blocks":[{"b":"code","lang":"ts","text":"send()"}]}]}"""
        ) as ServerMsg.Convo
        val code = msg.turns[0].blocks[0] as Block.Code
        assertEquals("ts", code.lang)
        assertNull(code.role)
    }

    // Additive: a header beside the marker, and a client that has never heard of `att` still
    // renders the marker it always did.
    @Test
    fun anAttachmentHeaderRidesBesideItsMarkerWithoutDisturbingTheBlock() {
        val msg = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"turns":[
               {"id":"t1","role":"user","blocks":[
                 {"b":"md","text":"[image · png]","att":{"id":"a7","kind":"image",
                   "mime":"image/png","bytes":52831,"name":"shot.png","thumb":{"w":8}}},
                 {"b":"md","text":"plain prose"}]}]}"""
        ) as ServerMsg.Convo
        val blocks = msg.turns[0].blocks
        val marked = blocks[0] as Block.Md
        assertEquals("[image · png]", marked.text)
        assertEquals("a7", marked.att?.id)
        assertEquals("image", marked.att?.kind)
        assertEquals(52831L, marked.att?.bytes)
        assertEquals("shot.png", marked.att?.name)
        assertNull((blocks[1] as Block.Md).att)
    }

    // A kind nobody here has heard of, no mime, no name, no size: still a handle, because the
    // handle is the only field the fetch needs.
    @Test
    fun anAttachmentIsOnlyAsGoodAsItsIdAndThatIsEnough() {
        val msg = Wire.decode(
            """{"t":"convo","pane":"p","cursor":"c","more":false,"turns":[
               {"id":"t1","role":"user","blocks":[
                 {"b":"md","text":"[audio]","att":{"id":"a9","kind":"audio"}},
                 {"b":"md","text":"[image]","att":{"kind":"image"}},
                 {"b":"md","text":"[image]","att":"a10"}]}]}"""
        ) as ServerMsg.Convo
        val blocks = msg.turns[0].blocks
        assertEquals("a9", (blocks[0] as Block.Md).att?.id)
        assertEquals("audio", (blocks[0] as Block.Md).att?.kind)
        assertNull((blocks[1] as Block.Md).att, "an att with no handle is not an attachment")
        assertNull((blocks[2] as Block.Md).att, "an att that is not an object is not an attachment")
        assertEquals(3, blocks.size, "a header this client cannot use took its whole turn down")
    }

    @Test
    fun errorAndPongDecode() {
        val failure = Wire.decode("""{"t":"error","code":"not_writer","message":"read-only","pane":null}""")
        assertEquals("not_writer", (failure as ServerMsg.Failure).code)
        assertNull(failure.pane)
        assertEquals(7, (Wire.decode("""{"t":"pong","n":7}""") as ServerMsg.Pong).n)
    }

    @Test
    fun clientMessagesEncodeToTheDocumentedShape() {
        assertEquals(
            """{"t":"watch","pane":"p","scrollback":true,"conversation":true}""",
            Wire.encode(ClientMsg.Watch("p")),
        )
        assertEquals(
            "{\"t\":\"input\",\"pane\":\"p\",\"text\":\"\\u001b[5~\"}",
            Wire.encode(ClientMsg.InputText("p", "\u001b[5~")),
        )
        assertEquals(
            """{"t":"input","pane":"p","keys":["ctrl+c"]}""",
            Wire.encode(ClientMsg.InputKeys("p", listOf("ctrl+c"))),
        )
        assertEquals("""{"t":"answer","pane":"p","key":"1"}""", Wire.encode(ClientMsg.Answer("p", "1")))
        assertEquals("""{"t":"resync"}""", Wire.encode(ClientMsg.Resync))
    }

    // `served` says whether this node reaches that session as a node of its own. Additive, so a
    // node — or a peer relayed through a hub on an older build — that says nothing serves what it
    // lists, and is never wrongly drawn as unreachable.
    @Test
    fun capsSaysWhichOfItsSessionsThisNodeActuallyServes() {
        val caps = Wire.decode(
            """{"t":"caps","node":"01JA","agent_kinds":["claude"],"sessions":[
               {"name":"default","running":true,"served":true},
               {"name":"agents","running":true,"served":false},
               {"name":"legacy","running":false}]}"""
        ) as ServerMsg.NodeCaps
        assertEquals(listOf(true, false, true), caps.sessions.map { it.served })
    }

    // A node reports the release that supersedes its own build. The field is additive on the
    // wire, so a herd from a node that has never heard of it must still decode.
    @Test
    fun aNodeEntryCarriesTheReleaseThatSupersedesIt() {
        val herd = Wire.decode(
            """{"t":"herd","panes":[],"nodes":[
               {"id":"01JA","name":"front","kind":"local","online":true,"build":"0.1.0","update":"0.1.2"},
               {"id":"01JB","name":"back","kind":"peer","online":true,"build":"0.1.2"}]}"""
        ) as ServerMsg.Herd
        assertEquals("0.1.2", herd.nodes[0].update)
        assertNull(herd.nodes[1].update, "a node that said nothing was read as saying something")
    }
}
