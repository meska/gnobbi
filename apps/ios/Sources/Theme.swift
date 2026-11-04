import SwiftUI

// Colours and measures come only from DesignTokens (generated from crates/matrix-domain/design/tokens.toml).
struct Palette {
    let surface, text, secondary, accent, border, selected, error, onAccent: Color
    static func color(_ hex: UInt32) -> Color {
        Color(red: Double((hex >> 16) & 0xff) / 255, green: Double((hex >> 8) & 0xff) / 255, blue: Double(hex & 0xff) / 255)
    }
    static let light = Palette(
        surface: color(DesignTokens.Classic.surface), text: color(DesignTokens.Classic.text),
        secondary: color(DesignTokens.Classic.secondary), accent: color(DesignTokens.Classic.accent),
        border: color(DesignTokens.Classic.border), selected: color(DesignTokens.Classic.selected),
        error: color(DesignTokens.Classic.error), onAccent: color(DesignTokens.Classic.onAccent))
    static let dark = Palette(
        surface: color(DesignTokens.Dark.surface), text: color(DesignTokens.Dark.text),
        secondary: color(DesignTokens.Dark.secondary), accent: color(DesignTokens.Dark.accent),
        border: color(DesignTokens.Dark.border), selected: color(DesignTokens.Dark.selected),
        error: color(DesignTokens.Dark.error), onAccent: color(DesignTokens.Dark.onAccent))
    static func current(_ scheme: ColorScheme) -> Palette { scheme == .dark ? dark : light }
}
