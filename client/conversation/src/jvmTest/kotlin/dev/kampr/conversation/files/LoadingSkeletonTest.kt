package dev.kampr.conversation.files

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlinx.coroutines.awaitCancellation
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals

private const val NODE = "01JNODE"
private const val ANCHOR = "01JNODE/w3:p2"

private object Silent : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes = awaitCancellation()
}

private val FOLDER = FilesPlace.Dir(NODE, ANCHOR, "/home/u/dev")

@OptIn(ExperimentalTestApi::class)
class LoadingSkeletonTest {
    // The node never answers, so the pulse never ends: the clock is stepped by hand, since an
    // auto-advancing one waits for an idle that never comes.
    private fun frames(place: FilesPlace, still: Boolean): List<Int> {
        val seen = mutableListOf<Int>()
        runComposeUiTest {
            mainClock.autoAdvance = false
            setContent { CompositionLocalProvider(LocalReduceMotion provides still) { Stage(Silent, Stack(place)) } }
            mainClock.advanceTimeByFrame()
            onNode(hasText("asking the node", substring = true)).assertDoesNotExist()
            repeat(3) {
                mainClock.advanceTimeBy(450)
                seen += onNode(hasTestTag(SKELETON_TAG)).captureToImage().toPixelMap().buffer.contentHashCode()
            }
        }
        return seen
    }

    private fun showsASkeleton(place: FilesPlace) = frames(place, still = true).let { assertEquals(3, it.size) }

    @Test
    fun aFolderThatHasNotArrivedIsASkeleton() = showsASkeleton(FOLDER)

    @Test
    fun aFileThatHasNotArrivedIsASkeleton() = showsASkeleton(FilesPlace.File(NODE, ANCHOR, "/home/u/dev/a.txt"))

    @Test
    fun aRepositoryThatHasNotArrivedIsASkeleton() = showsASkeleton(FilesPlace.Repo(NODE, ANCHOR, "/home/u/dev"))

    @Test
    fun theSkeletonBreathes() = assertNotEquals(1, frames(FOLDER, still = false).toSet().size)

    @Test
    fun theSkeletonHoldsStillForAReaderWhoAskedForLessMotion() = assertEquals(1, frames(FOLDER, still = true).toSet().size)
}
