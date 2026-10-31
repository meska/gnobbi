package chat.gnobbi.mobile

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import chat.gnobbi.design.DesignTokens

/** Colours and measures come only from DesignTokens (generated from crates/matrix-domain/design/tokens.toml). */
class Palette(val surface: Long, val text: Long, val secondary: Long, val accent: Long, val border: Long, val error: Long, val onAccent: Long) {
    val surfaceColor get() = c(surface)
    val textColor get() = c(text)
    val secondaryColor get() = c(secondary)
    val accentColor get() = c(accent)
    val borderColor get() = c(border)
    val errorColor get() = c(error)
    val onAccentColor get() = c(onAccent)
    private fun c(hex: Long) = Color(0xFF000000L or hex)

    companion object {
        val light = DesignTokens.Classic.let { Palette(it.SURFACE, it.TEXT, it.SECONDARY, it.ACCENT, it.BORDER, it.ERROR, it.ON_ACCENT) }
        val dark = DesignTokens.Dark.let { Palette(it.SURFACE, it.TEXT, it.SECONDARY, it.ACCENT, it.BORDER, it.ERROR, it.ON_ACCENT) }
        @Composable fun current() = if (isSystemInDarkTheme()) dark else light
    }
}
