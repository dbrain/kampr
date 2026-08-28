// A test task's cache key is its classpath, not the machine that ran it — so a result stored by
// one runner is restored onto another with different fonts, a different locale and no display,
// and reported as a pass without a single test executing. Four grid tests failed on every runner
// for weeks behind `:terminal:jvmTest FROM-CACHE`. CI applies this so a green client job means
// the tests ran; nothing here changes a local build.
gradle.allprojects {
    tasks.withType(AbstractTestTask::class.java).configureEach {
        outputs.upToDateWhen { false }
        outputs.doNotCacheIf("a result from another machine is not evidence this one ran") { true }
    }
}
