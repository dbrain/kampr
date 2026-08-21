package dev.kampr.shared.net

// `hello.security.installable` has been rendered as "install to home screen" on the setup ladder
// since phase 6 with no listener behind it anywhere in the client. A browser fires
// `beforeinstallprompt` once, early, and refuses to prompt outside a user gesture — so the event
// is caught before wasm boots and this is what reaches it.
interface InstallPrompt {
    val available: Boolean

    suspend fun prompt(): Boolean
}

expect fun createInstallPrompt(): InstallPrompt

// Nothing to install: a desktop JAR and an APK are already installed.
class NoInstallPrompt : InstallPrompt {
    override val available: Boolean = false
    override suspend fun prompt(): Boolean = false
}
