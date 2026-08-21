package dev.kampr.shared

import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.WATCH_FALL_MS
import dev.kampr.shared.model.WATCH_NOTICE_MS
import dev.kampr.shared.model.WATCH_RISE_MS
import dev.kampr.shared.model.WatchPresence
import dev.kampr.shared.model.othersWatching
import dev.kampr.shared.model.watchersNotice
import dev.kampr.shared.model.watchersPhrase
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

// A `herd` as a node actually sends it: `watchers` is omitted on the pane only this client is on,
// and carried on the two that something else also has open.
private val HERD_FRAME = """
    {"t":"herd",
     "nodes":[{"id":"01JNODE","name":"comingclean","kind":"local","online":true,"rtt_ms":0.4,
               "herdr_version":"0.8.2","build":"0.1.0+abc1234"}],
     "panes":[
       {"id":"01JNODE/w3:p1","node_id":"01JNODE","workspace":"kampr","tab":"1",
        "cwd":"/home/dbrain/dev/kampr","agent":"claude","agent_status":"working",
        "cols":74,"rows":30,"scrollback_rows":900,"has_conversation":true,
        "updated_at":"2026-08-21T13:44:02Z"},
       {"id":"01JNODE/w3:p2","node_id":"01JNODE","workspace":"kampr","tab":"2",
        "cwd":"/home/dbrain/dev/kampr","agent_status":"idle","rows":24,"watchers":2,
        "updated_at":"2026-08-21T13:44:02Z"},
       {"id":"01JNODE/w4:p1","node_id":"01JNODE","workspace":"herdr","tab":"1",
        "agent":"claude","agent_status":"blocked","rows":24,"watchers":5,
        "updated_at":"2026-08-21T13:44:02Z"}]}
""".trimIndent()

class WatchersTest {
    @Test
    fun theHerdFrameCarriesTheWatchCountAndItsAbsenceMeansNobodyElse() {
        val herd = assertNotNull(Wire.decode(HERD_FRAME) as? ServerMsg.Herd, "the herd frame did not decode")
        val (alone, shared, crowded) = Triple(herd.panes[0], herd.panes[1], herd.panes[2])

        assertNull(alone.watchers, "a pane with no `watchers` key must not invent one")
        assertEquals(0, othersWatching(alone), "an absent count claimed somebody else was there")
        assertEquals(2, shared.watchers)
        assertEquals(1, othersWatching(shared))
        assertEquals(5, crowded.watchers)
        assertEquals(4, othersWatching(crowded))
        // Absent and 1 are the same fact, and a node is allowed to send either.
        assertEquals(othersWatching(alone), othersWatching(alone.copy(watchers = 1)))
        assertEquals(0, othersWatching(null))
    }

    @Test
    fun aHerdPatchCanRaiseAndDropTheCountOnAPaneAlreadyKnown() {
        val herd = Herd().applyPatch(
            Wire.decode(
                """{"t":"herd.patch",
                    "added":{"nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],
                             "panes":[{"id":"01JNODE/w3:p2","node_id":"01JNODE","rows":24}]},
                    "changed":{},"removed_ids":[]}"""
            ) as ServerMsg.HerdPatch
        )
        assertEquals(0, othersWatching(herd.panes.single()))

        val joined = herd.applyPatch(
            Wire.decode(
                """{"t":"herd.patch","added":{},
                    "changed":{"panes":[{"id":"01JNODE/w3:p2","node_id":"01JNODE","rows":24,"watchers":3}]},
                    "removed_ids":[]}"""
            ) as ServerMsg.HerdPatch
        )
        assertEquals(2, othersWatching(joined.panes.single()))

        val left = joined.applyPatch(
            Wire.decode(
                """{"t":"herd.patch","added":{},
                    "changed":{"panes":[{"id":"01JNODE/w3:p2","node_id":"01JNODE","rows":24}]},
                    "removed_ids":[]}"""
            ) as ServerMsg.HerdPatch
        )
        assertEquals(0, othersWatching(left.panes.single()), "dropping the key must drop the count")
    }

    // What the number means to a person: another client has this pane open. Not that anyone is
    // typing, and — because a hub relays a whole crowd behind one watch — never a headcount.
    @Test
    fun theWordsSayAnotherClientHasItOpenAndNeverThatSomebodyIsTyping() {
        assertNull(watchersTag(0))
        assertNull(watchersPhrase(0))
        assertNull(watchersNotice(0))

        assertEquals("also open", watchersTag(1))
        assertEquals("also open · 3", watchersTag(3))

        assertEquals("also open on another client", watchersPhrase(1))
        assertTrue(
            watchersPhrase(3)!!.contains("at least 3 other clients"),
            "a relayed count can undercount, so a number must be spoken as a floor: ${watchersPhrase(3)}",
        )

        for (n in 1..4) {
            val notice = assertNotNull(watchersNotice(n))
            assertTrue(notice.first().isUpperCase(), "a live region reads a sentence: $notice")
            assertTrue(notice.contains("open"), notice)
            assertFalse(
                Regex("""\b(is|are) typing\b""").containsMatchIn(notice),
                "watching is not typing, and the copy must not claim it is: $notice",
            )
            assertTrue(notice.contains("not necessarily typing"), "the notice must say what it is not: $notice")
        }
    }

    // A join is adopted only once it has held. Our own socket re-watching before the node has
    // reaped the dead one shows up as a second watcher for a moment, and a client that shouted
    // about that would be shouting about itself.
    @Test
    fun aJoinIsAdoptedOnlyAfterItHolds() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        assertEquals(0, presence.others, "a count adopted on arrival has no way to reject a blip")
        assertNull(presence.notice)

        presence.tick(WATCH_RISE_MS - 1)
        assertEquals(0, presence.others)
        assertNull(presence.notice)

        presence.tick(WATCH_RISE_MS)
        assertEquals(1, presence.others)
        assertEquals(watchersNotice(1), presence.notice)
    }

    @Test
    fun aWatcherThatComesAndGoesInsideTheRiseWindowIsNeverSurfaced() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        assertEquals(0, presence.others, "a watcher that was never really there reached the screen")
        assertNull(presence.notice, "a watcher that was never really there was announced")
        presence.tick(WATCH_RISE_MS / 2)
        assertEquals(0, presence.others, "a watcher that was never really there reached the screen")
        assertNull(presence.notice, "a watcher that was never really there was announced")
        presence.observe(0, WATCH_RISE_MS / 2)
        presence.tick(WATCH_RISE_MS * 4)
        assertEquals(0, presence.others)
        assertNull(presence.notice)
        assertFalse(presence.pending())
    }

    @Test
    fun theNoticeClearsItselfWithoutTheCountChanging() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        presence.tick(WATCH_RISE_MS)
        assertNotNull(presence.notice)
        presence.tick(WATCH_RISE_MS + WATCH_NOTICE_MS - 1)
        assertNotNull(presence.notice, "the notice went before it could be read")
        presence.tick(WATCH_RISE_MS + WATCH_NOTICE_MS)
        assertNull(presence.notice, "the notice stayed up as a permanent badge")
        assertEquals(1, presence.others, "clearing the notice must not clear the fact")
        assertFalse(presence.pending(), "a settled pane must not keep asking to be ticked")
    }

    @Test
    fun aSocketBlipNeitherClearsTheCountNorAnnouncesItTwice() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        presence.tick(WATCH_RISE_MS)
        val settled = WATCH_RISE_MS + WATCH_NOTICE_MS
        presence.tick(settled)
        assertNull(presence.notice)

        presence.observe(0, settled)
        presence.tick(settled + WATCH_FALL_MS - 1)
        assertEquals(1, presence.others, "a reconnecting socket emptied the pane")

        presence.observe(1, settled + WATCH_FALL_MS - 1)
        presence.tick(settled + WATCH_FALL_MS * 3)
        assertEquals(1, presence.others)
        assertNull(presence.notice, "a client that never left was announced as arriving")
    }

    @Test
    fun aRealDepartureClearsAndALaterJoinIsAnnouncedAgain() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        presence.tick(WATCH_RISE_MS)
        val left = WATCH_RISE_MS + WATCH_NOTICE_MS

        presence.observe(0, left)
        presence.tick(left + WATCH_FALL_MS - 1)
        assertEquals(1, presence.others)
        presence.tick(left + WATCH_FALL_MS)
        assertEquals(0, presence.others, "a client that really went stayed on the screen")
        assertNull(presence.notice)

        val back = left + WATCH_FALL_MS
        presence.observe(1, back)
        presence.tick(back + WATCH_RISE_MS)
        assertEquals(1, presence.others)
        assertEquals(watchersNotice(1), presence.notice, "the second arrival went unannounced")
    }

    @Test
    fun aSecondArrivalOnTopOfTheFirstIsItsOwnNotice() {
        val presence = WatchPresence()
        presence.observe(1, 0)
        presence.tick(WATCH_RISE_MS)
        val two = WATCH_RISE_MS + WATCH_NOTICE_MS
        presence.tick(two)

        presence.observe(2, two)
        assertEquals(1, presence.others, "the second arrival was adopted before it had held")
        assertNull(presence.notice, "the second arrival was announced before it had held")
        presence.tick(two + WATCH_RISE_MS - 1)
        assertEquals(1, presence.others, "the second arrival was adopted before it had held")
        presence.tick(two + WATCH_RISE_MS)
        assertEquals(2, presence.others)
        assertEquals(watchersNotice(2), presence.notice)
    }
}
