rootProject.name = "kampr-client"

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
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
include(":androidApp")
include(":desktopApp")
include(":webApp")

include(":terminal-spike")
include(":terminal-spike:androidApp")
