package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent

internal const val A = "01JKAMPRNODE0000000000000/w1:p1"
internal const val B = "01JKAMPRNODE0000000000000/w1:p2"

internal fun List<ClientMsg>.manages(): List<ClientMsg.Manage> = filterIsInstance<ClientMsg.Manage>()

internal fun List<ClientMsg>.sizings(pane: String): List<ManageOp.PaneSize> =
    manages().mapNotNull { it.request as? ManageOp.PaneSize }.filter { it.at == pane }

internal fun List<ManageOp.PaneSize>.modes() = map { it.mode }

internal fun took(rid: String?) = ServerMsg.Managed(op = ManageOp.PaneSize.OP, ok = true, id = null, rid = rid)

internal fun refused(rid: String?) = ServerMsg.Managed(
    op = ManageOp.PaneSize.OP,
    ok = false,
    id = null,
    rid = rid,
    code = "herdr",
    message = "pane already has an attached client",
)

// A claim and the node's answer to it, which is the only thing that makes one a hold. `take` is
// false where the node refused — a contended controller (#21), a peer link that dropped under the
// op — which is a thing that happens and not a thing to assume away.
@OptIn(ExperimentalCoroutinesApi::class)
internal suspend fun TestScope.claim(
    holds: MatchHolds,
    sent: List<ClientMsg>,
    pane: String,
    cols: Int,
    rows: Int,
    take: Boolean = true,
): Boolean {
    val claim = async { holds.claim(pane, cols, rows) }
    runCurrent()
    val rid = sent.manages().lastOrNull()?.rid
    holds.acked(if (take) took(rid) else refused(rid))
    return claim.await()
}
