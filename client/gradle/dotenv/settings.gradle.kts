rootProject.name = "dotenv"

// Same reason as the main build's settings: Gradle 10 refuses to auto-provision a JDK without a
// toolchain repository, and jvmToolchain(21) is auto-provisioned on any machine whose system JDK
// is not 21.
plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "1.0.0"
}

dependencyResolutionManagement {
    repositories {
        mavenCentral()
    }
}
