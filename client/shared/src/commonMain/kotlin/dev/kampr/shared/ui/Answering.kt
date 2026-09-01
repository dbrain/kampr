package dev.kampr.shared.ui

import dev.kampr.shared.model.ConnectionStatus

// An answer is a keystroke on the wire — `KamprConnection.typing` lists it beside `input`, and for
// the same reason: a digit queued over a dead socket is pressed into whatever dialog is up twenty
// seconds later. So it is dropped where it stands, and every surface that sends one has to say
// that rather than offer a control that presses and delivers nothing.
//
// Four of them do: the conversation's question card, the terminal's chip row, the fleet board's
// answer chips, and the reply box — whose `submit` clears the field, so the sentence goes with it.
//
// `live` is false for the whole reconnect ladder, which is where a phone is every time it is
// opened on a blocked-agent notification: the card is drawn from memory while the socket climbs
// its backoff. `undelivered` is the other half — the socket was live when the chip was pressed and
// died with the frame still queued, and `discardTyping` counted it lost.
data class Answering(val enabled: Boolean, val note: String?) {
    companion object {
        val Ready = Answering(enabled = true, note = null)
    }
}

fun answering(status: ConnectionStatus, undelivered: Int): Answering = when {
    status !is ConnectionStatus.Live ->
        Answering(false, "not connected — this cannot leave the device yet")
    undelivered > 0 ->
        Answering(true, "that did not get through — try it again")
    else -> Answering.Ready
}
