import com.android.build.api.variant.AndroidComponentsExtension
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
}

// CMP 1.11.1's resource plugin emits no Android assets for a
// com.android.kotlin.multiplatform.library target, so the APK stages them itself.
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

val stageComposeResources = tasks.register<StageComposeResources>("stageComposeResourceAssets") {
    source.set(file("../src/commonMain/composeResources"))
    resourcePackage.set("dev.kampr.terminal.spike.res")
    outputDir.set(layout.buildDirectory.dir("generated/composeResourceAssets"))
}

extensions.getByType(AndroidComponentsExtension::class.java).onVariants { variant ->
    variant.sources.assets?.addGeneratedSourceDirectory(
        stageComposeResources,
        StageComposeResources::outputDir,
    )
}

android {
    namespace = "dev.kampr.terminal.spike.app"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "dev.kampr.terminal.spike"
        minSdk = libs.versions.android.minSdk.get().toInt()
        targetSdk = libs.versions.android.targetSdk.get().toInt()
        versionCode = 1
        versionName = "0.1"
    }

    buildTypes {
        getByName("release") { isMinifyEnabled = false }
    }

    kotlin {
        jvmToolchain(21)
    }
}

dependencies {
    implementation(project(":terminal-spike"))
    implementation(libs.androidx.activity.compose)
    implementation(libs.compose.runtime)
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
}
