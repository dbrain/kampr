package dev.kampr.shared.model

import dev.kampr.shared.wire.ServerMsg

// A pane id is `<node_id>/<local>`, so the node a pane belongs to is readable off the id itself —
// which is what this has to do, because the herd entry for a pane on a node that has just gone
// offline is the first thing to disappear.
fun nodeOfPane(paneId: String): String = paneId.substringBefore('/')

// Whether a refusal is worth interrupting the operator for, and only this half can answer it: the
// node knows what went wrong and has no idea what is on the screen.
//
// **The rule the operator gave: a disconnection is loud only when it is the thing they are using.**
// A node going unreachable used to arrive with no subject at all, so it was drawn the only way it
// could be — a strip over whatever screen was open — and a node nobody was looking at interrupted a
// pane on a different one, on a phone. Everything quiet here is still said: `nodes[].online` and
// `nodes[].detail` draw it on the herd screen, and a pane-scoped refusal is held on the pane it is
// about until the operator arrives there.
//
// A failure naming neither is the connection itself — auth, a refusal, a revocation — and there is
// nowhere quieter for those to go.
fun saidOutLoud(failure: ServerMsg.Failure, paneOnScreen: String?): Boolean = when {
    failure.pane != null -> failure.pane == paneOnScreen
    failure.node != null -> failure.node == paneOnScreen?.let(::nodeOfPane)
    else -> true
}
