package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ProtocolUpdateTest {
    @Test
    fun deviceTokenUsesTheExactSubprotocolSpelling() {
        assertEquals("kampr.token.abc123", Endpoint("http://n", "abc123").subprotocol)
        assertNull(Endpoint("http://n").subprotocol)
    }

    @Test
    fun securityDrivesAffordancesAndTierZeroHidesPasskeys() {
        val hello = Wire.decode(
            """{"t":"hello","protocol":1,"node_id":"01J","node_name":"cc","build":"b","role":"full",
               "caps":{"push":true,"scrollback":true,"conversation":true,"manage":true},
               "security":{"tier":0,"encrypted":false,"unencrypted_banner":true,"passkeys":false,
                           "push":false,"installable":false,
                           "unlocks":["passkeys","push","installable"]}}"""
        ) as ServerMsg.Hello
        assertTrue(hello.caps.manage)
        assertEquals(0, hello.security.tier)
        assertFalse(hello.security.passkeys)
        assertTrue(hello.security.unencryptedBanner)
        assertEquals(listOf("passkeys", "push", "installable"), hello.security.unlocks)
    }

    @Test
    fun aHelloWithoutSecurityStillDecodesToTheSafeDefault() {
        val hello = Wire.decode(
            """{"t":"hello","protocol":1,"node_id":"01J","node_name":"cc","build":"b","role":"full",
               "caps":{}}"""
        ) as ServerMsg.Hello
        assertFalse(hello.security.passkeys)
        assertTrue(hello.security.unencryptedBanner)
    }

    @Test
    fun aNullQuestionClearsThePrompt() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"pending","pane":"p","question":"Approve edit to server.ts",
                   "options":[{"key":"1","label":"Yes"}],"source":"screen"}"""
            )!!
        )
        assertNotNull(store.pane("p").pending)
        store.accept(Wire.decode("""{"t":"pending","pane":"p","question":null,"options":[]}""")!!)
        assertNull(store.pane("p").pending)
    }

    @Test
    fun prefsRoundTripPerPane() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"prefs","panes":{"01J/w3:p2":{"zoom":1.6,"view":"terminal"},
                                          "01J/w3:p1":{"zoom":0.7}}}"""
            )!!
        )
        assertEquals(1.6f, store.prefsFor("01J/w3:p2").zoom)
        assertEquals("terminal", store.prefsFor("01J/w3:p2").view)
        assertEquals(0.7f, store.prefsFor("01J/w3:p1").zoom)
        assertNull(store.prefsFor("unknown").zoom)

        assertEquals(
            """{"t":"prefs","pane":"p","prefs":{"view":"terminal"}}""",
            Wire.encode(ClientMsg.SetPrefs("p", mapOf("view" to "terminal"))),
        )
    }

    @Test
    fun anUnrecognisedErrorCodeStillCarriesItsMessage() {
        val failure = Wire.decode(
            """{"t":"error","code":"quota_exhausted","message":"try again tomorrow","pane":"p"}"""
        ) as ServerMsg.Failure
        assertEquals("quota_exhausted", failure.code)
        assertEquals("try again tomorrow", failure.message)
        for (code in listOf("unsupported", "not_found", "not_writer")) {
            val decoded = Wire.decode("""{"t":"error","code":"$code","message":"m"}""")
            assertEquals(code, (decoded as ServerMsg.Failure).code)
        }
    }

    @Test
    fun scrollbackTailAppendsWithoutShrinkingTheRing() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":100,
                   "rows":[{"row":100,"runs":[{"s":0,"x":"a"}]},{"row":101,"runs":[{"s":0,"x":"b"}]}],
                   "total_rows":2,"complete":true,"capped":false}"""
            )!!
        )
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":100,
                   "rows":[{"row":102,"runs":[{"s":0,"x":"c"}]}],
                   "total_rows":1,"complete":true,"capped":false}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertEquals(3, scrollback.historyRows)
        assertEquals("a", scrollback.row(100)?.runs?.first()?.x)
        assertEquals("c", scrollback.row(102)?.runs?.first()?.x)
    }

    @Test
    fun aDiscardAdvancesFromTopAndDropsWhatCameBefore() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":0,
                   "rows":[{"row":0,"runs":[{"s":0,"x":"a"}]},{"row":1,"runs":[{"s":0,"x":"b"}]}],
                   "total_rows":2,"complete":true,"capped":false}"""
            )!!
        )
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":5000,
                   "rows":[{"row":5000,"runs":[{"s":0,"x":"z"}]}],
                   "total_rows":1,"complete":false,"capped":true}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertEquals(5000, scrollback.fromTop)
        assertEquals(1, scrollback.historyRows)
        assertNull(scrollback.row(0))
        assertTrue(scrollback.capped)
    }
}
