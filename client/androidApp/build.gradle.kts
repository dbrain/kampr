import com.android.build.api.variant.AndroidComponentsExtension
import java.util.Properties
import java.util.zip.ZipFile
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
}

// CMP 1.11.1 emits no Android assets for a com.android.kotlin.multiplatform.library
// target, so the APK stages :shared's composeResources itself (probe #64).
abstract class StageComposeResources : DefaultTask() {
    @get:InputDirectory
    abstract val source: DirectoryProperty

    @get:Input
    abstract val resourcePackage: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @get:Inject
    abstract val fs: FileSystemOperations

    @TaskAction
    fun stage() {
        fs.sync {
            from(source)
            into(outputDir.get().dir("composeResources").dir(resourcePackage.get()))
        }
    }
}

// Probe #64 is silent when it regresses — the app just falls back to system sans. So the
// packaged artefact is read back and every staged resource has to be inside it.
abstract class VerifyPackagedResources : DefaultTask() {
    @get:InputFiles
    abstract val artefacts: ConfigurableFileCollection

    @get:InputDirectory
    abstract val source: DirectoryProperty

    @get:Input
    abstract val assetPrefix: Property<String>

    @get:Input
    abstract val entryPrefix: Property<String>

    @get:OutputFile
    abstract val report: RegularFileProperty

    @TaskAction
    fun verify() {
        val root = source.get().asFile
        val expected = root.walkTopDown()
            .filter { it.isFile }
            .map { "${assetPrefix.get()}/${it.relativeTo(root).invariantSeparatorsPath}" }
            .toSortedSet()
        require(expected.isNotEmpty()) { "No composeResources found under $root" }

        val packaged = artefacts.files.filter { it.isFile && it.extension in setOf("apk", "aab") }
        require(packaged.isNotEmpty()) { "No packaged artefact to verify" }

        val report = report.get().asFile
        report.parentFile.mkdirs()
        val lines = packaged.map { file ->
            val entries = ZipFile(file).use { zip -> zip.entries().toList().map { it.name }.toSet() }
            val missing = expected.filterNot { "${entryPrefix.get()}$it" in entries }
            if (missing.isNotEmpty()) {
                throw GradleException(
                    "${file.name} is missing ${missing.size} of ${expected.size} compose resources — " +
                        "probe #64 has regressed:\n" + missing.joinToString("\n") { "  $it" },
                )
            }
            "${file.name}: all ${expected.size} compose resources packaged"
        }
        report.writeText(lines.joinToString("\n") + "\n")
        lines.forEach { logger.lifecycle(it) }
    }
}

val composeResourceRoot = file("../shared/src/commonMain/composeResources")
val composeResourcePackage = "dev.kampr.shared.res"

val stageComposeResources = tasks.register<StageComposeResources>("stageComposeResourceAssets") {
    source.set(composeResourceRoot)
    resourcePackage.set(composeResourcePackage)
    outputDir.set(layout.buildDirectory.dir("generated/composeResourceAssets"))
}

extensions.getByType(AndroidComponentsExtension::class.java).onVariants { variant ->
    variant.sources.assets?.addGeneratedSourceDirectory(
        stageComposeResources,
        StageComposeResources::outputDir,
    )
}

// version.properties is the Gradle root's, because that is the file the kobup helper bumps.
val versionProps = Properties().apply {
    providers.fileContents(rootProject.layout.projectDirectory.file("version.properties"))
        .asText.get().reader().use { load(it) }
}

// Release signing never lives in the repo: a Gradle property (~/.gradle/gradle.properties)
// wins, an environment variable is the CI fallback.
fun configValue(property: String, environment: String): Provider<String> =
    providers.gradleProperty(property).orElse(providers.environmentVariable(environment))

val releaseStoreFile = configValue("kamprReleaseStoreFile", "KAMPR_RELEASE_STORE_FILE")
    .map { rootProject.file(it) }
val releaseStorePassword = configValue("kamprReleaseStorePassword", "KAMPR_RELEASE_STORE_PASSWORD")
val releaseKeyAlias = configValue("kamprReleaseKeyAlias", "KAMPR_RELEASE_KEY_ALIAS")
val releaseKeyPassword = configValue("kamprReleaseKeyPassword", "KAMPR_RELEASE_KEY_PASSWORD")

android {
    namespace = "dev.kampr.app"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "dev.kampr.app"
        // minSdk 26 buys the full java.time/NIO surface without desugaring, adaptive icons
        // and every Android TV box worth sideloading onto, at ~2% of live devices.
        minSdk = libs.versions.android.minSdk.get().toInt()
        targetSdk = libs.versions.android.targetSdk.get().toInt()
        versionCode = versionProps.getProperty("versionCode").trim().toInt()
        versionName = versionProps.getProperty("versionName").trim()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        create("release") {
            if (releaseStoreFile.isPresent && releaseStoreFile.get().isFile) {
                storeFile = releaseStoreFile.get()
                storePassword = releaseStorePassword.orNull
                keyAlias = releaseKeyAlias.orNull
                keyPassword = releaseKeyPassword.orNull
            }
            // v3 is what makes key rotation possible at all; minSdk 26 makes v1 dead weight.
            enableV1Signing = false
            enableV2Signing = true
            enableV3Signing = true
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            signingConfig = signingConfigs.getByName("release")
        }
    }

    packaging {
        resources.excludes += setOf(
            "/META-INF/{AL2.0,LGPL2.1}",
            "/META-INF/*.version",
            "DebugProbesKt.bin",
            "kotlin-tooling-metadata.json",
        )
    }

    kotlin {
        jvmToolchain(21)
    }
}

val checkReleaseSigning = tasks.register("checkReleaseSigning") {
    val store = releaseStoreFile
    val storePass = releaseStorePassword
    val alias = releaseKeyAlias
    val keyPass = releaseKeyPassword
    doLast {
        val missing = buildList {
            if (!store.isPresent) add("kamprReleaseStoreFile / KAMPR_RELEASE_STORE_FILE")
            if (!storePass.isPresent) add("kamprReleaseStorePassword / KAMPR_RELEASE_STORE_PASSWORD")
            if (!alias.isPresent) add("kamprReleaseKeyAlias / KAMPR_RELEASE_KEY_ALIAS")
            if (!keyPass.isPresent) add("kamprReleaseKeyPassword / KAMPR_RELEASE_KEY_PASSWORD")
        }
        if (missing.isNotEmpty()) {
            throw GradleException(
                "Release signing is not configured — refusing to build an unsigned or debug-signed APK.\n" +
                    "Missing:\n" + missing.joinToString("\n") { "  $it" } +
                    "\n\nRun `make android-keystore`, then see docs/07-android-release.md.",
            )
        }
        if (!store.get().isFile) {
            throw GradleException(
                "Release keystore ${store.get()} does not exist. Restore it from backup — " +
                    "a different key can never update an already-installed Kampr.",
            )
        }
    }
}

val verifyReleaseApk = tasks.register<VerifyPackagedResources>("verifyReleaseApkResources") {
    source.set(composeResourceRoot)
    assetPrefix.set("composeResources/$composeResourcePackage")
    entryPrefix.set("assets/")
    artefacts.from(layout.buildDirectory.dir("outputs/apk/release").map { it.asFileTree })
    report.set(layout.buildDirectory.file("reports/composeResources/release-apk.txt"))
}

val verifyReleaseBundle = tasks.register<VerifyPackagedResources>("verifyReleaseBundleResources") {
    source.set(composeResourceRoot)
    assetPrefix.set("composeResources/$composeResourcePackage")
    entryPrefix.set("base/assets/")
    artefacts.from(layout.buildDirectory.dir("outputs/bundle/release").map { it.asFileTree })
    report.set(layout.buildDirectory.file("reports/composeResources/release-bundle.txt"))
}

tasks.matching { it.name in setOf("validateSigningRelease", "packageRelease", "packageReleaseBundle") }
    .configureEach { dependsOn(checkReleaseSigning) }
tasks.matching { it.name == "assembleRelease" }.configureEach { finalizedBy(verifyReleaseApk) }
tasks.matching { it.name == "bundleRelease" }.configureEach { finalizedBy(verifyReleaseBundle) }

dependencies {
    implementation(project(":shared"))
    implementation(project(":terminal"))
    implementation(project(":conversation"))
    implementation(project(":mosaic"))
    implementation(libs.androidx.activity.compose)
    implementation(libs.compose.runtime)
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.runner)
    // The one question about a paste on Android that no host test can reach: whether the platform
    // routes a real clipboard image to `contentReceiver` (#369). It needs a device and a Compose
    // tree, so it needs the instrumented harness.
    androidTestImplementation(libs.compose.ui.test)
    androidTestImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)
}

// `./gradlew :androidApp:publishToKobup`. The helper reads .kobup.json and version.properties
// from ${rootDir}, which for this build is client/, not the repository root.
val kobupHelper = configValue("kobupHelperPath", "KOBUP_HELPER")
    .map { file(it) }
    .orElse(rootProject.layout.projectDirectory.dir("../../tinyfiddler/kob/kobup/gradle").file("publish-to-kobup.gradle").asFile)
if (kobupHelper.get().isFile) {
    apply(from = kobupHelper.get())
} else {
    tasks.register("publishToKobup") {
        val path = kobupHelper.get()
        doLast {
            throw GradleException(
                "kobup helper not found at $path. Set -PkobupHelperPath=/path/to/kobup/gradle/publish-to-kobup.gradle",
            )
        }
    }
}
