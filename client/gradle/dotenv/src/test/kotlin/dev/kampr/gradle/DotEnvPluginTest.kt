package dev.kampr.gradle

import org.gradle.testkit.runner.GradleRunner
import org.gradle.testkit.runner.TaskOutcome
import java.io.File
import kotlin.io.path.createTempDirectory
import kotlin.test.AfterTest
import kotlin.test.BeforeTest
import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertEquals

// The only consumer of `env` is the kobup publish helper, which lives outside this repo and is
// applied with `apply(from = …)`. These tests reproduce its call shape verbatim rather than
// calling the extension directly, because the shape is the contract.
class DotEnvPluginTest {
    private lateinit var projectDir: File

    private val helper = """
        def capturedToken = null
        try { capturedToken = env.fetchOrNull("KOBUP_TOKEN") } catch (ignored) {}

        tasks.register("reportToken") {
            def captured = capturedToken
            doLast {
                println "captured=" + captured
                def token = captured ?: System.getenv("KOBUP_TOKEN")
                if (!token) throw new GradleException("KOBUP_TOKEN not set. Add it to .env or export it.")
                println "token=" + token
            }
        }
    """.trimIndent()

    @BeforeTest
    fun setUp() {
        projectDir = createTempDirectory("dotenv-test").toFile()
        projectDir.resolve("settings.gradle.kts").writeText(
            """
            plugins { id("dev.kampr.dotenv") }
            rootProject.name = "fixture"
            include(":androidApp")
            """.trimIndent(),
        )
        projectDir.resolve("helper.gradle").writeText(helper)
        projectDir.resolve("androidApp").mkdirs()
        projectDir.resolve("androidApp/build.gradle.kts")
            .writeText("""apply(from = rootProject.file("helper.gradle"))""")
    }

    @AfterTest
    fun tearDown() {
        projectDir.deleteRecursively()
    }

    private fun runner(vararg env: Pair<String, String>): GradleRunner =
        GradleRunner.create()
            .withProjectDir(projectDir)
            .withPluginClasspath()
            .withEnvironment(System.getenv() - "KOBUP_TOKEN" + env)
            .withArguments(":androidApp:reportToken", "--configuration-cache", "--stacktrace")

    @Test
    fun `a value in dot env reaches the helper`() {
        projectDir.resolve(".env").writeText("# publishing\nKOBUP_TOKEN=from-dot-env\n")

        val result = runner().build()

        assertEquals(TaskOutcome.SUCCESS, result.task(":androidApp:reportToken")?.outcome)
        assertContains(result.output, "captured=from-dot-env")
        assertContains(result.output, "token=from-dot-env")
    }

    @Test
    fun `an exported variable reaches the helper with no dot env present`() {
        val result = runner("KOBUP_TOKEN" to "from-environment").build()

        assertEquals(TaskOutcome.SUCCESS, result.task(":androidApp:reportToken")?.outcome)
        assertContains(result.output, "captured=from-environment")
    }

    @Test
    fun `dot env wins over an exported variable`() {
        projectDir.resolve(".env").writeText("KOBUP_TOKEN=from-dot-env\n")

        val result = runner("KOBUP_TOKEN" to "from-environment").build()

        assertContains(result.output, "captured=from-dot-env")
    }

    @Test
    fun `quoted and commented dot env lines are handled`() {
        projectDir.resolve(".env").writeText(
            """
            # a comment
            
            KOBUP_TOKEN="quoted value"
            OTHER=untouched
            """.trimIndent(),
        )

        assertContains(runner().build().output, "captured=quoted value")
    }

    @Test
    fun `an absent dot env is not an error and the helper says what to do`() {
        val result = runner().buildAndFail()

        assertContains(result.output, "captured=null")
        assertContains(result.output, "KOBUP_TOKEN not set. Add it to .env or export it.")
    }
}
