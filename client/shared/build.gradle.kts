import org.gradle.api.tasks.PathSensitivity
import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.compose.multiplatform)
    alias(libs.plugins.android.kmp.library)
}

kotlin {
    jvmToolchain(21)

    applyDefaultHierarchyTemplate {
        common {
            group("skiko") {
                withJvm()
                withWasmJs()
            }
        }
    }

    android {
        namespace = "dev.kampr.shared"
        compileSdk = libs.versions.android.compileSdk.get().toInt()
        minSdk = libs.versions.android.minSdk.get().toInt()
        compilerOptions { jvmTarget.set(JvmTarget.JVM_21) }
        // Without this the Android actuals are the one set nothing in `allTests` ever runs, which
        // is how `defaultEndpoint()` shipped pointing at the emulator's alias for its own host.
        withHostTest {}
    }

    jvm()

    @OptIn(ExperimentalWasmDsl::class)
    wasmJs {
        outputModuleName.set("kamprShared")
        browser()
    }

    sourceSets {
        commonMain.dependencies {
            implementation(libs.compose.runtime)
            implementation(libs.compose.foundation)
            implementation(libs.compose.ui)
            implementation(libs.compose.ui.backhandler)
            implementation(libs.compose.components.resources)
            implementation(libs.kotlinx.coroutines.core)
            implementation(libs.kotlinx.serialization.json)
            implementation(libs.ktor.client.core)
            implementation(libs.ktor.client.websockets)
            implementation(libs.ktor.client.content.negotiation)
            implementation(libs.ktor.serialization.json)
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
            implementation(libs.kotlinx.coroutines.test)
        }
        jvmTest.dependencies {
            implementation(compose.desktop.currentOs)
            implementation(libs.compose.ui.test)
            implementation(libs.navigationevent)
        }
        androidMain.dependencies {
            implementation(libs.ktor.client.okhttp)
            implementation(libs.androidx.activity.compose)
            // Credential Manager is the only authenticator API Android has. `-play-services-auth`
            // is what carries it below API 34, where the framework has no provider of its own.
            implementation(libs.androidx.credentials)
            implementation(libs.androidx.credentials.play.services)
            // CameraX for the preview and the frame pump; zxing-core reads the frames. `core` is
            // pure Java, so it adds no `.so` to a universal APK — see docs/07-android-release.md.
            implementation(libs.androidx.camera.camera2)
            implementation(libs.androidx.camera.lifecycle)
            implementation(libs.androidx.camera.view)
            implementation(libs.zxing.core)
            // A distributor's endpoint is an RFC 8291 endpoint, so the node's sender is unchanged:
            // no Google project, no `google-services.json`, no per-app secret (docs/08-notifications.md).
            api(libs.unifiedpush.connector)
        }
        jvmMain.dependencies {
            implementation(libs.ktor.client.cio)
        }
        wasmJsMain.dependencies {
            implementation(libs.kotlinx.browser)
            implementation(libs.ktor.client.js)
        }
    }
}

compose.resources {
    publicResClass = true
    packageOfResClass = "dev.kampr.shared.res"
}

// `SemanticsLayerTest` and `TokenLayerTest` read the *other* modules' sources off disk, which
// Gradle cannot see — so the task was up to date, and skipped, on every build where only those
// modules changed. Which is exactly the build where the guards have something to say: a raw tap
// gesture added to the conversation shipped green locally and failed on CI, where nothing was
// cached. Naming the directories they walk is what makes the local gate the same gate.
tasks.named<Test>("jvmTest") {
    inputs.files(
        listOf("shared", "terminal", "conversation", "mosaic")
            .map { rootProject.file("$it/src/commonMain") }
            .filter { it.isDirectory },
    ).withPropertyName("surfaceSources").withPathSensitivity(PathSensitivity.RELATIVE)
}
