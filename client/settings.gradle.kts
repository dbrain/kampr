rootProject.name = "kampr-client"

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

// Gradle 10 refuses to auto-provision a JDK without a toolchain repository, and this build's
// jvmToolchain(21) is auto-provisioned on any machine whose system JDK is not 21.
plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "1.0.0"
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

include(":shared")
include(":terminal")
include(":conversation")
include(":mosaic")
include(":androidApp")
include(":desktopApp")
include(":webApp")

include(":terminal-spike")
include(":terminal-spike:androidApp")
