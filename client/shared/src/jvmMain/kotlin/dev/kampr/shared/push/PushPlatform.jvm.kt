package dev.kampr.shared.push

actual fun createPushPlatform(): PushPlatform = NoPush()
