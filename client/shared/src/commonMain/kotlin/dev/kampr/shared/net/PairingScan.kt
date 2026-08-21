package dev.kampr.shared.net

import androidx.compose.runtime.Composable

// Scanning the desktop's pairing QR from inside the app.
//
// An Android app link cannot do this job: a link declares its hosts in the manifest at build time
// and every operator's node is at a different one, so a scanned link opens the browser client and
// the installed app can never be the thing that was enrolled. A camera in the app can, and it
// needs no host to be known in advance.
//
// Absent where there is no camera to ask for — which is every desktop and the browser build, where
// the phone's own camera app already opens the link.
expect val pairingScanAvailable: Boolean

// Full-screen while it is up. `onScanned` fires once with whatever the symbol said; deciding
// whether that text is a node is [pairingFrom]'s job, not the camera's.
@Composable
expect fun PairingScanSurface(onScanned: (String) -> Unit, onClose: () -> Unit)
