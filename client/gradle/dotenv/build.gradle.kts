plugins {
    `kotlin-dsl`
}

group = "dev.kampr.gradle"

kotlin {
    jvmToolchain(21)
}

gradlePlugin {
    plugins.create("dotenv") {
        id = "dev.kampr.dotenv"
        implementationClass = "dev.kampr.gradle.DotEnvPlugin"
    }
}

dependencies {
    testImplementation(kotlin("test"))
    testImplementation(gradleTestKit())
}

tasks.test {
    useJUnitPlatform()
}
