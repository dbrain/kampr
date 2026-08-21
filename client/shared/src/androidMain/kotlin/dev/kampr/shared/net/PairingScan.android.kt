package dev.kampr.shared.net

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.LifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.announce
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.delay

actual val pairingScanAvailable: Boolean = true

@Composable
actual fun PairingScanSurface(onScanned: (String) -> Unit, onClose: () -> Unit) {
    val context = LocalContext.current
    val owner = KamprHost.activity as? LifecycleOwner
    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    // A refusal is an answer, not a dead end: the address and the code can still be typed, and
    // saying so is the difference between a considered no and a broken screen.
    var refused by remember { mutableStateOf(false) }
    var found by remember { mutableStateOf<String?>(null) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        granted = allowed
        refused = !allowed
    }
    LaunchedEffect(Unit) {
        if (!granted) ask.launch(Manifest.permission.CAMERA)
    }
    // Long enough to see that the symbol was read, short enough not to be a wait. Without it the
    // screen simply vanishes and nothing ever says the camera did its job.
    LaunchedEffect(found) {
        val text = found ?: return@LaunchedEffect
        delay(450)
        onScanned(text)
    }
    ImmersiveWhileScanning()

    val tokens = Kampr.tokens
    Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
        if (granted && owner != null) {
            CameraPreview(owner) { text -> if (found == null) found = text }
            Viewfinder(found != null)
            Column(
                Modifier.fillMaxSize().padding(horizontal = 22.dp, vertical = 28.dp),
                verticalArrangement = Arrangement.SpaceBetween,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                KText(
                    if (found == null) "Point this at the QR on the other screen." else "Got it.",
                    tokens.type.body,
                    if (found == null) Color.White else tokens.color.done,
                    Modifier.announce(
                        if (found == null) "Point the camera at the pairing code on the other screen"
                        else "Pairing code read",
                    ),
                    maxLines = 2,
                )
                QuietAction("Cancel", onClose, Modifier.fillMaxWidth(), label = "Stop scanning")
            }
        } else {
            Column(
                Modifier.padding(28.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                KText(
                    if (refused) {
                        "Kampr cannot use the camera. Type the node's address and the pairing " +
                            "code it printed instead — that path works exactly the same."
                    } else {
                        "Waiting for the camera…"
                    },
                    tokens.type.body,
                    Color.White,
                    Modifier.announce(
                        if (refused) "Camera refused. Type the address and code instead." else "",
                    ),
                    maxLines = 4,
                )
                PrimaryAction("Back", onClose, Modifier.fillMaxWidth(), label = "Back to typing the address")
            }
        }
    }
}

// The status bar over a viewfinder is somebody else's furniture in the middle of aiming a camera.
@Composable
private fun ImmersiveWhileScanning() {
    val activity = KamprHost.activity
    DisposableEffect(activity) {
        val window = activity?.window
        val controller = window?.let { WindowInsetsControllerCompat(it, it.decorView) }
        val wasLight = controller?.isAppearanceLightStatusBars
        controller?.hide(WindowInsetsCompat.Type.systemBars())
        controller?.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        // A camera cannot know what it is about to be pointed at, so the clock and the battery
        // are drawn light for as long as it is up — dark icons over a dark room are unreadable,
        // and a swipe brings the bars back transiently whatever this does.
        controller?.isAppearanceLightStatusBars = false
        controller?.isAppearanceLightNavigationBars = false
        onDispose {
            controller?.show(WindowInsetsCompat.Type.systemBars())
            wasLight?.let {
                controller.isAppearanceLightStatusBars = it
                controller.isAppearanceLightNavigationBars = it
            }
        }
    }
}

// Everything outside the square is dimmed and the corners are drawn, because a raw camera feed
// says nothing about where to aim or whether the thing is even looking.
@Composable
private fun Viewfinder(found: Boolean) {
    val tokens = Kampr.tokens
    val edge = if (found) tokens.color.done else Color.White
    Canvas(Modifier.fillMaxSize()) {
        val side = minOf(size.width, size.height) * 0.72f
        val left = (size.width - side) / 2f
        val top = (size.height - side) / 2f
        val shade = Color.Black.copy(alpha = 0.45f)
        drawRect(shade, Offset.Zero, Size(size.width, top))
        drawRect(shade, Offset(0f, top + side), Size(size.width, size.height - top - side))
        drawRect(shade, Offset(0f, top), Size(left, side))
        drawRect(shade, Offset(left + side, top), Size(size.width - left - side, side))

        val arm = side * 0.12f
        val stroke = Stroke(width = 4.dp.toPx())
        val corners = listOf(
            Triple(Offset(left, top), Offset(arm, 0f), Offset(0f, arm)),
            Triple(Offset(left + side, top), Offset(-arm, 0f), Offset(0f, arm)),
            Triple(Offset(left, top + side), Offset(arm, 0f), Offset(0f, -arm)),
            Triple(Offset(left + side, top + side), Offset(-arm, 0f), Offset(0f, -arm)),
        )
        for ((corner, across, down) in corners) {
            drawLine(edge, corner, corner + across, strokeWidth = stroke.width)
            drawLine(edge, corner, corner + down, strokeWidth = stroke.width)
        }
    }
}

@Composable
private fun CameraPreview(owner: LifecycleOwner, onScanned: (String) -> Unit) {
    val context = LocalContext.current
    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    // One symbol ends the scan. Without this the analyser keeps firing while the surface tears
    // down and enrolment is attempted several times over.
    val seen = remember { AtomicBoolean(false) }
    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { viewContext ->
            val view = PreviewView(viewContext)
            val future = ProcessCameraProvider.getInstance(viewContext)
            future.addListener({
                val provider = future.get()
                val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(analysisExecutor) { image ->
                    val text = try {
                        decodeQr(image)
                    } finally {
                        image.close()
                    }
                    if (text != null && seen.compareAndSet(false, true)) {
                        view.post { onScanned(text) }
                    }
                }
                runCatching {
                    provider.unbindAll()
                    provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
                }
            }, ContextCompat.getMainExecutor(viewContext))
            view
        },
    )
    DisposableEffect(context) {
        onDispose {
            runCatching { ProcessCameraProvider.getInstance(context).get().unbindAll() }
            analysisExecutor.shutdown()
        }
    }
}

private fun decodeQr(image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val luma = ByteArray(buffer.remaining())
    buffer.get(luma)
    return decodeQrLuminance(luma, plane.rowStride, image.width, image.height)
}

// The whole of the decode, in terms a test can build without a camera. A frame's `rowStride` is
// padded up to a hardware alignment and is not the width — treating it as one shears the image
// and nothing ever scans.
//
// `QRCodeReader` rather than `MultiFormatReader`: it is the only format Kampr prints, and naming
// it directly is what lets R8 drop every other reader out of the APK.
fun decodeQrLuminance(luma: ByteArray, rowStride: Int, width: Int, height: Int): String? {
    if (width <= 0 || height <= 0 || rowStride < width || luma.size < rowStride * height) return null
    val source = PlanarYUVLuminanceSource(luma, rowStride, height, 0, 0, width, height, false)
    val reader = QRCodeReader()
    return try {
        reader.decode(BinaryBitmap(HybridBinarizer(source))).text
    } catch (notFound: NotFoundException) {
        null
    } catch (other: ReaderException) {
        null
    } finally {
        reader.reset()
    }
}
