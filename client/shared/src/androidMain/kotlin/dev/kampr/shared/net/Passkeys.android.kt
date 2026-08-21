package dev.kampr.shared.net

import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import java.security.MessageDigest

// Android's authenticator is Credential Manager, not `navigator.credentials`. It runs a ceremony
// for a native app only where the relying party — the operator's own node — serves a Digital Asset
// Links file naming this package and this signing certificate, which is why the node serves one
// (docs/07-android-release.md). What it signs into the client data is not an `https://` origin but
// `android:apk-key-hash:…`, which is why the node allows that origin too.
private class CredentialManagerPasskeys : Passkeys {
    // Two things have to be true, and the Activity is the one that changes: without it there is
    // nothing to raise the system sheet from, and a button that opens nothing is worse than none.
    override val available: Boolean
        get() = KamprHost.activity != null

    override val platform: String = "android"

    override val identity: AppIdentity?
        get() = KamprHost.activity?.let(::identityOf)

    override suspend fun create(optionsJson: String): PasskeyResult {
        val activity = KamprHost.activity ?: return NO_ACTIVITY
        val request = credentialManagerRequest(optionsJson) ?: return UNREADABLE
        return try {
            val response = CredentialManager.create(activity)
                .createCredential(activity, CreatePublicKeyCredentialRequest(request))
            when (response) {
                is CreatePublicKeyCredentialResponse -> PasskeyResult.Ok(response.registrationResponseJson)
                else -> PasskeyResult.Failed("Android returned ${response.type}, which is not a passkey.")
            }
        } catch (cancelled: CreateCredentialCancellationException) {
            PasskeyResult.Cancelled
        } catch (failure: CreateCredentialException) {
            PasskeyResult.Failed(reasonFor(failure.type, failure.errorMessage?.toString()))
        }
    }

    override suspend fun get(optionsJson: String): PasskeyResult {
        val activity = KamprHost.activity ?: return NO_ACTIVITY
        val request = credentialManagerRequest(optionsJson) ?: return UNREADABLE
        return try {
            val response = CredentialManager.create(activity).getCredential(
                activity,
                GetCredentialRequest(listOf(GetPublicKeyCredentialOption(request))),
            )
            when (val credential = response.credential) {
                is PublicKeyCredential -> PasskeyResult.Ok(credential.authenticationResponseJson)
                else -> PasskeyResult.Failed("Android returned ${credential.type}, which is not a passkey.")
            }
        } catch (cancelled: GetCredentialCancellationException) {
            PasskeyResult.Cancelled
        } catch (none: NoCredentialException) {
            PasskeyResult.Failed("This phone holds no passkey for that node yet. Pair with a code first, then add one.")
        } catch (failure: GetCredentialException) {
            PasskeyResult.Failed(reasonFor(failure.type, failure.errorMessage?.toString()))
        }
    }

    // Credential Manager's own message is the useful half when there is one; its type is a class
    // name and reads as one. Whether the node's asset links are the real cause is answered by
    // `PasskeyApi`, which is the half of this that can read the node.
    private fun reasonFor(type: String, message: String?): String =
        message?.takeIf { it.isNotBlank() } ?: "Android refused the passkey ($type)."

    private companion object {
        val NO_ACTIVITY = PasskeyResult.Failed("Kampr is not on screen, so Android has nothing to ask on.")
        val UNREADABLE = PasskeyResult.Failed("This node's challenge was not one Android could read.")
    }
}

// The same SHA-256 that goes in `assetlinks.json`, read off the certificate this build is actually
// signed with rather than off a constant — a debug build and a build from source are each signed
// with something else, and the node has to be told which.
private fun identityOf(activity: Activity): AppIdentity? = runCatching {
    val packages = activity.packageManager
    val signatures = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        val info = packages.getPackageInfo(activity.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        info.signingInfo?.let { if (it.hasMultipleSigners()) it.apkContentsSigners else it.signingCertificateHistory }
    } else {
        @Suppress("DEPRECATION")
        packages.getPackageInfo(activity.packageName, PackageManager.GET_SIGNATURES).signatures
    }
    val certificate = signatures?.firstOrNull() ?: return null
    val digest = MessageDigest.getInstance("SHA-256").digest(certificate.toByteArray())
    AppIdentity(activity.packageName, digest.joinToString(":") { "%02X".format(it) })
}.getOrNull()

actual fun createPasskeys(): Passkeys = CredentialManagerPasskeys()
