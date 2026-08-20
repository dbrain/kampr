package dev.kampr.terminal.bench

expect fun emitBench(line: String)

expect val platformLabel: String

expect fun graphicsBackend(): String
