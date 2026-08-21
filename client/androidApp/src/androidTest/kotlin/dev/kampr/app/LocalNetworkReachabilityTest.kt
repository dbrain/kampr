package dev.kampr.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import java.net.HttpURLConnection
import java.net.URL
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test

// Every self-hosted node this app is pointed at is a private address over plain http —
// `Endpoint.schemeFor` picks http for exactly those. From targetSdk 37 a node on the phone's own
// subnet needs ACCESS_LOCAL_NETWORK, and without it connect() times out silently rather than
// failing as a permission error, so its absence is invisible until a user cannot connect.
class LocalNetworkReachabilityTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val node: String? = InstrumentationRegistry.getArguments().getString("kamprNode")

    // A user grants this from the dialog `MainActivity.askForPermissions` raises; an install for
    // tests does not, and an ungranted API 37 device cannot reach the node at all.
    @Before
    fun grantLocalNetworkAccess() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.CINNAMON_BUN) return
        instrumentation.uiAutomation
            .grantRuntimePermission(context.packageName, Manifest.permission.ACCESS_LOCAL_NETWORK)
    }

    @Test
    fun theNodeAnswersOverPlainHttp() {
        assumeTrue("no -e kamprNode <url> given, so there is no node to reach", node != null)
        val connection = (URL("$node/api/node").openConnection() as HttpURLConnection).apply {
            connectTimeout = 10_000
            readTimeout = 10_000
        }
        val body = try {
            assertEquals(200, connection.responseCode)
            connection.inputStream.bufferedReader().use { it.readText() }
        } finally {
            connection.disconnect()
        }
        assertTrue("$node did not answer as a Kampr node: $body", "\"node_id\"" in body)
    }

    // A test cannot switch the restriction on for itself: PlatformCompat kills the process that
    // `am compat enable RESTRICT_LOCAL_NETWORK <pkg>` names. Run that from a shell before
    // instrumenting to see the real refusal; this asserts the declaration it depends on.
    @Test
    fun theAppAsksForLocalNetworkAccess() {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.CINNAMON_BUN)
        val requested = context.packageManager
            .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            .orEmpty()
        assertTrue(
            "ACCESS_LOCAL_NETWORK is not declared — every node on the phone's own subnet would " +
                "time out with no error to show the user",
            Manifest.permission.ACCESS_LOCAL_NETWORK in requested,
        )
    }
}
