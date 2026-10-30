package dev.kampr.shared

import dev.kampr.shared.ui.BackArm
import dev.kampr.shared.ui.BrowserHistory
import kotlin.test.Test
import kotlin.test.assertEquals

// A browser's history as the page sees it: entries the app pushed carry the arm, the one it was
// opened on does not, and a traversal is answered later by a popstate rather than at once.
private class FakeHistory(armedAtLoad: Boolean = false) : BrowserHistory {
    val entries = mutableListOf(false).apply { if (armedAtLoad) add(true) }
    var index = entries.lastIndex
    var left = false
    private val pending = ArrayDeque<Int>()
    lateinit var arm: BackArm

    override fun push() {
        while (entries.lastIndex > index) entries.removeAt(entries.lastIndex)
        entries += true
        index = entries.lastIndex
    }

    override fun back() = traverse(-1)

    fun pressBack() = traverse(-1)

    fun pressForward() = traverse(+1)

    // Each traversal is resolved against wherever the previous one landed, as the HTML spec queues
    // them, so two `back()`s before either lands go back two.
    private fun traverse(delta: Int) {
        pending += delta
    }

    fun settle() {
        while (pending.isNotEmpty()) {
            val to = index + pending.removeFirst()
            if (to < 0) {
                left = true
                continue
            }
            if (to > entries.lastIndex) continue
            index = to
            arm.popped(entries[index])
        }
    }
}

private class Page(var depth: Int, armedAtLoad: Boolean = false) {
    val history = FakeHistory(armedAtLoad)
    var backs = 0
    val arm = BackArm(history, armedAtLoad, wanted = { depth > 0 }, back = {
        backs++
        depth--
    }).also { history.arm = it }

    fun change(to: Int) {
        depth = to
        arm.sync()
        history.settle()
    }

    fun pressBack() {
        history.pressBack()
        history.settle()
    }
}

class BrowserBackTest {
    @Test
    fun atTheRootNothingIsPushedAndBackLeavesTheSite() {
        val page = Page(depth = 0)
        page.arm.sync()
        assertEquals(1, page.history.entries.size, "a root with nothing to go back to pushes nothing")
        page.pressBack()
        assertEquals(true, page.history.left)
        assertEquals(0, page.backs)
    }

    @Test
    fun eachBrowserBackRunsTheAppsBackOnceUntilTheRootThenLeaves() {
        val page = Page(depth = 0)
        page.change(3)
        assertEquals(2, page.history.entries.size, "one armed entry, however deep the app is")
        page.pressBack()
        assertEquals(1, page.backs)
        assertEquals(2, page.depth)
        assertEquals(2, page.history.entries.size, "re-armed while there is still somewhere to go")
        page.pressBack()
        page.pressBack()
        assertEquals(3, page.backs)
        assertEquals(0, page.depth)
        assertEquals(false, page.history.left)
        assertEquals(0, page.history.index)
        page.pressBack()
        assertEquals(true, page.history.left, "at the root the browser back leaves")
        assertEquals(3, page.backs)
    }

    @Test
    fun goingBackInsideTheAppTakesTheArmBackSoTheNextBrowserBackLeaves() {
        val page = Page(depth = 0)
        page.change(1)
        page.change(0)
        assertEquals(0, page.history.index, "the armed entry is consumed")
        assertEquals(0, page.backs, "the echo of that traversal is not a back")
        page.pressBack()
        assertEquals(true, page.history.left)
    }

    @Test
    fun forwardOntoAStaleArmAtTheRootIsUndone() {
        val page = Page(depth = 0)
        page.change(1)
        page.pressBack()
        assertEquals(0, page.depth)
        page.history.pressForward()
        page.history.settle()
        assertEquals(0, page.history.index, "nothing to go back to, so the arm is stepped off again")
        assertEquals(1, page.backs)
        page.pressBack()
        assertEquals(true, page.history.left)
    }

    @Test
    fun reopeningWhileTheArmIsBeingTakenBackStillArms() {
        val page = Page(depth = 0)
        page.change(1)
        page.depth = 0
        page.arm.sync()
        page.depth = 1
        page.arm.sync()
        page.history.settle()
        assertEquals(1, page.history.index, "armed again once the traversal landed")
        page.pressBack()
        assertEquals(1, page.backs)
        assertEquals(0, page.depth)
    }

    @Test
    fun theArmIsTakenBackOnceHoweverOftenTheRootIsReportedBeforeTheTraversalLands() {
        val page = Page(depth = 0)
        page.change(1)
        page.depth = 0
        page.arm.sync()
        page.arm.sync()
        page.history.settle()
        assertEquals(false, page.history.left, "a second step back would have left the site")
        assertEquals(0, page.history.index)
    }

    @Test
    fun aReloadOnTheArmedEntryAtTheRootStepsOffIt() {
        val page = Page(depth = 0, armedAtLoad = true)
        page.arm.sync()
        page.history.settle()
        assertEquals(0, page.history.index)
        page.pressBack()
        assertEquals(true, page.history.left)
    }

    @Test
    fun aReloadOnTheArmedEntryWithSomewhereToGoKeepsIt() {
        val page = Page(depth = 1, armedAtLoad = true)
        page.arm.sync()
        assertEquals(2, page.history.entries.size, "no second arm stacked on the first")
        page.pressBack()
        assertEquals(1, page.backs)
    }
}
