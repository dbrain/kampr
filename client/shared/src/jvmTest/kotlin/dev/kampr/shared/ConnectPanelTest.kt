package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.ui.ConnectPanel
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.Security
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private const val NODE = "http://192.168.1.24:8790"

// What `kampr init` prints: eight characters of the confusable-free alphabet, grouped in fours.
private const val CODE = "2KQK-RB5Y"

// The first run on a phone reaches this panel and nothing else, so every affordance it needs has
// to be here — reachable by name, not merely painted.
@OptIn(ExperimentalTestApi::class)
class ConnectPanelTest {
    @Test
    fun aBareHostAndPortIsAllThatHasToBeTyped() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(null, null, { dialled = it })
                }
            }
        }
        onNodeWithContentDescription("Connect to this node").assertIsNotEnabled()
        onNodeWithContentDescription("Node address").performTextInput("192.168.1.24:8790")
        onNodeWithContentDescription("Connect to this node").assertIsEnabled().performClick()
        waitForIdle()
        assertEquals(Endpoint(NODE), dialled)
    }

    @Test
    fun anAddressUsedBeforeIsOneTap() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(null, null, { dialled = it }, recent = listOf("https://kampr.example.com"))
                }
            }
        }
        onNodeWithContentDescription("Use https://kampr.example.com").performClick()
        onNodeWithContentDescription("Connect to this node").performClick()
        waitForIdle()
        assertEquals(Endpoint("https://kampr.example.com"), dialled)
    }

    // A code scanned off the desktop's QR arrives armed but not spent: the tap is still the
    // operator's, and the enrolment is still one they can see happening. A code that is already
    // complete when the panel opens must not spend itself on the way in.
    @Test
    fun aScannedCodeArrivesInTheFieldAndIsRedeemedOnTheTap() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(Endpoint(NODE), null, { dialled = it }, offeredCode = CODE)
                }
            }
        }
        assertNull(dialled, "a scan must not enrol on its own")
        onNodeWithContentDescription("Connect to this node").performClick()
        waitForIdle()
        assertEquals(Endpoint(NODE, CODE), dialled)
    }

    // The report: pairing works, but the code field does nothing on its own — the operator has to
    // find a key. A pairing code is eight characters long and the node is the one that says so, so
    // the field knows when it is holding a whole one and there is nothing left to ask.
    @Test
    fun aCodeTypedToItsFullLengthEnrolsWithNothingElseTouched() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(Endpoint(NODE), null, { dialled = it })
                }
            }
        }
        val field = onNodeWithContentDescription("Pairing code, only when pairing")
        for (character in CODE) {
            assertNull(dialled, "enrolled at '$character', before the code was whole")
            field.performTextInput(character.toString())
            waitForIdle()
        }
        assertEquals(Endpoint(NODE, CODE), dialled)
    }

    // A phone offers to paste as readily as it offers to type, and a paste arrives as one edit
    // rather than eight.
    @Test
    fun aPastedCodeEnrolsTheSameWayATypedOneDoes() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(Endpoint(NODE), null, { dialled = it })
                }
            }
        }
        onNodeWithContentDescription("Pairing code, only when pairing").performTextInput(CODE)
        waitForIdle()
        assertEquals(Endpoint(NODE, CODE), dialled)
    }

    // Half a code is a code being typed, not a code being offered.
    @Test
    fun aPartialCodeIsLeftAlone() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(Endpoint(NODE), null, { dialled = it })
                }
            }
        }
        onNodeWithContentDescription("Pairing code, only when pairing").performTextInput("2KQK")
        waitForIdle()
        assertNull(dialled, "four characters is not a pairing code")
    }

    // The other half of the report: whatever the operator does reach for, the keyboard's own
    // action key has to be it. Both fields answer it, because either one can be the last thing
    // touched before the node is dialled.
    @Test
    fun theKeyboardsOwnActionDialsTheNode() = runComposeUiTest {
        for (field in listOf("Node address", "Pairing code, only when pairing")) {
            var dialled: Endpoint? = null
            setContent {
                Bars {
                    Box(Modifier.size(420.dp, 900.dp)) {
                        ConnectPanel(null, null, { dialled = it })
                    }
                }
            }
            onNodeWithContentDescription("Node address").performTextInput("192.168.1.24:8790")
            onNodeWithContentDescription(field).performImeAction()
            waitForIdle()
            assertEquals(Endpoint(NODE), dialled, "$field did not answer the keyboard's action key")
        }
    }

    // The whole point of the change: with nothing stored and nothing derivable, the screen the
    // app opens on is the one that asks — not a herd behind a failure.
    @Test
    fun theSetupScreenAsksForAnAddressWhenThereIsNone() = runComposeUiTest {
        setContent {
            Bars {
                Box(Modifier.size(420.dp, 900.dp)) {
                    SetupScreen(
                        status = null,
                        security = Security(),
                        running = false,
                        endpoint = null,
                        nodes = emptyList(),
                        pairingCode = null,
                        pairingError = null,
                        onConnect = {},
                        onPairingCode = {},
                        onDevices = {},
                        onAppearance = {},
                        onNotifications = {},
                    )
                }
            }
        }
        onNodeWithContentDescription("Node address").assertExists()
        onNodeWithContentDescription("Connect to this node").assertExists()
    }
}
