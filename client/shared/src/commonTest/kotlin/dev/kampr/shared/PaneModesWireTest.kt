package dev.kampr.shared

import dev.kampr.shared.wire.PaneModes
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

// docs/26: a native pane says what its program asked of the mouse; a herdr pane and an older node
// say nothing, and absent is not the same as everything off.
class PaneModesWireTest {
    private val frame = """
        {"t":"herd","nodes":[],"panes":[
          {"id":"n/n:1","node_id":"n","rows":24,"provider":"native","agent":"claude",
           "modes":{"mouse":"drag","mouse_sgr":true,"alt_scroll":true,"app_cursor":false,"alt_screen":true}},
          {"id":"n/w1:p1","node_id":"n","rows":24,"agent":"claude"},
          {"id":"n/n:2","node_id":"n","rows":24,"modes":{"mouse":"a-future-word"}}]}
    """.trimIndent()

    @Test
    fun aPaneCarriesItsModesAndOneWithoutThemCarriesNone() {
        val herd = assertNotNull(Wire.decode(frame) as? ServerMsg.Herd)
        assertEquals(
            PaneModes(mouse = "drag", mouseSgr = true, altScroll = true, appCursor = false, altScreen = true),
            herd.panes[0].modes,
        )
        assertNull(herd.panes[1].modes)
        assertEquals(true, herd.panes[2].modes?.tracksMouse, "a mouse word this client has not heard of is still a mouse")
    }
}
