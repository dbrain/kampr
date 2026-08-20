package dev.kampr.shared.platform

import java.awt.Toolkit

// There is no portable desktop API for this. AWT republishes XSETTINGS under `gnome.` on X11 and
// XWayland, which covers GNOME and the toolkits that follow it; everything else falls back to the
// environment override, because guessing wrong here means an operator cannot switch it off.
actual fun reduceMotionSetting(): Boolean = runCatching {
    System.getenv("KAMPR_REDUCE_MOTION")?.takeIf { it.isNotBlank() }?.let { return it != "0" }
    val enabled = Toolkit.getDefaultToolkit().getDesktopProperty("gnome.Gtk/EnableAnimations")
    (enabled as? Number)?.toInt() == 0
}.getOrDefault(false)
