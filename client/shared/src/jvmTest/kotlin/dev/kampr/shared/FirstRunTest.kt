package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Screen
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The first run on a phone: nothing stored, and a platform that cannot derive an address from
// anything it is running on. It has to ask, not dial a guess and report the failure as an error.
class FirstRunTest {
    private fun state(prefs: MemoryPrefs = MemoryPrefs(), fallback: Endpoint? = null): Pair<AppState, CoroutineScope> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        return AppState(scope, KamprStore(), prefs, fallback) to scope
    }

    @Test
    fun withNothingStoredAndNothingToDeriveTheFirstScreenIsTheOneThatAsks() {
        val (app, scope) = state()
        try {
            assertNull(app.endpoint, "there is no address to dial")
            assertEquals(Screen.Setup, app.screen, "the first run must land where the address is asked for")
            app.start()
            assertEquals(ConnectionStatus.Idle, app.store.status.value, "nothing must be dialled")
        } finally {
            scope.cancel()
        }
    }

    // The browser is served by the node, so it can always derive an address — which is not the
    // same as being able to use one. Found in a real browser: a page that had never paired dialled
    // that derived address anyway, was refused eight times in twenty seconds, and sat on a herd it
    // could not fetch under "Reconnecting" with nothing offering to pair. `useEndpoint` has refused
    // to dial a tokenless address since 0.1.1; `start()` is the entry point that never learned.
    @Test
    fun aDerivedAddressWithNoEnrolmentIsOfferedButNotDialled() {
        val (app, scope) = state(fallback = Endpoint("http://127.0.0.1:8790"))
        try {
            assertEquals(
                "http://127.0.0.1:8790",
                app.endpoint?.baseUrl,
                "the derived address is still what the pairing panel should offer",
            )
            assertEquals(Screen.Setup, app.screen, "there is nothing to connect with, so it has to ask")
            app.start()
            assertEquals(ConnectionStatus.Idle, app.store.status.value, "a tokenless address must not be dialled")
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aStoredAddressWinsOverWhateverTheDeviceCouldDerive() {
        val prefs = MemoryPrefs()
        prefs.set("endpoint", "https://kampr.example.com")
        prefs.set("token", "tok")
        val (app, scope) = state(prefs, Endpoint("http://127.0.0.1:8790"))
        try {
            assertEquals(Endpoint("https://kampr.example.com", "tok"), app.endpoint)
            assertEquals(Screen.Herd, app.screen)
        } finally {
            scope.cancel()
        }
    }

    // The other half of the same rule: an enrolment that is there is dialled without being asked
    // about, which is the whole of "it remembered I was logged in".
    @Test
    fun aStoredEnrolmentIsDialledOnStartWithoutAskingAnything() {
        val prefs = MemoryPrefs()
        prefs.set("endpoint", "http://127.0.0.1:8790")
        prefs.set("token", "kmp_stored")
        val (app, scope) = state(prefs)
        try {
            assertEquals(Screen.Herd, app.screen)
            app.start()
            assertTrue(
                app.store.status.value != ConnectionStatus.Idle,
                "a device that is already paired must dial on its own",
            )
        } finally {
            scope.cancel()
        }
    }

    // Found in a real browser: Connect with an empty code and no enrolment stored adopted a null
    // token, dialled, was refused, and retried forever with nothing on screen saying why.
    @Test
    fun connectingWithNoCodeAndNoEnrolmentSaysSoRatherThanRetryingForever() {
        val (app, scope) = state()
        try {
            app.useEndpoint(Endpoint("http://192.168.1.24:8790"))
            assertNotNull(app.pairingError, "nothing said why it could not connect")
            assertNull(app.endpoint, "an address that cannot be used must not be adopted")
            assertEquals(ConnectionStatus.Idle, app.store.status.value)
        } finally {
            scope.cancel()
        }
    }

    // Somebody's node moves between a LAN address and a hostname, and typing the other one back
    // in from memory is the step that goes wrong.
    @Test
    fun theAddressesItHasReachedComeBack() {
        val prefs = MemoryPrefs()
        val (app, scope) = state(prefs)
        try {
            assertTrue(app.recentAddresses.isEmpty())
            app.rememberAddress("http://192.168.1.24:8790")
            app.rememberAddress("https://kampr.example.com")
            app.rememberAddress("http://192.168.1.24:8790")
            assertEquals(
                listOf("http://192.168.1.24:8790", "https://kampr.example.com"),
                app.recentAddresses,
                "most recent first, no duplicates",
            )
            repeat(8) { app.rememberAddress("http://host-$it:8790") }
            assertEquals(5, app.recentAddresses.size, "the list is a shortcut, not a history")

            val (reopened, second) = state(prefs)
            try {
                assertEquals(app.recentAddresses, reopened.recentAddresses, "it survives a restart")
            } finally {
                second.cancel()
            }
        } finally {
            scope.cancel()
        }
    }
}
