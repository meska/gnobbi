//! Every button icon of the app: Phosphor Icons 2.1.2 (MIT, https://phosphoricons.com), Regular for normal icons, Fill for active/selected
//! states, as two subset fonts (only the glyphs listed below) embedded in the binary. Nothing else draws icons.
//! Add an icon: `the phosphor script lookup <word>` prints the table line; paste it below and rerun `the phosphor script` to rebuild the fonts
//! (assets/fonts/Phosphor-subset.ttf, Phosphor-Fill-subset.ttf). The licence text is in THIRD_PARTY_NOTICES.md.
use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, RichText, Ui, Vec2};

macro_rules! icons {
    ($($name:ident = $cp:literal $css:literal),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Icon { $($name),* }
        impl Icon {
            pub const fn ch(self) -> char {
                match self { $(Icon::$name => char::from_u32($cp).unwrap()),* }
            }
        }
        #[cfg(test)]
        const ALL: &[Icon] = &[$(Icon::$name),*];
    };
}

icons! {
    ChatCircleDots = 0xe16c "chat-circle-dots",
    Chats = 0xe17c "chats",
    SlidersHorizontal = 0xe434 "sliders-horizontal",
    GearSix = 0xe272 "gear-six",
    CaretRight = 0xe13a "caret-right",
    CaretDown = 0xe136 "caret-down",
    CaretUp = 0xe13c "caret-up",
    CaretLeft = 0xe138 "caret-left",
    X = 0xe4f6 "x",
    Plus = 0xe3d4 "plus",
    PencilSimple = 0xe3b4 "pencil-simple",
    Check = 0xe182 "check",
    SignOut = 0xe42a "sign-out",
    ShieldCheck = 0xe40c "shield-check",
    Copy = 0xe1ca "copy",
    MagnifyingGlass = 0xe30c "magnifying-glass",
    Info = 0xe2ce "info",
    List = 0xe2f0 "list",
    Smiley = 0xe436 "smiley",
    ChartBarHorizontal = 0xe152 "chart-bar-horizontal",
    Paperclip = 0xe39a "paperclip",
    Microphone = 0xe326 "microphone",
    PaperPlaneRight = 0xe396 "paper-plane-right",
    Play = 0xe3d0 "play",
    Pause = 0xe39e "pause",
    DownloadSimple = 0xe20c "download-simple",
    ArrowDown = 0xe03e "arrow-down",
    Record = 0xe3ee "record",
    Desktop = 0xe560 "desktop",
    DeviceMobile = 0xe1e0 "device-mobile",
    Globe = 0xe288 "globe",
    Users = 0xe4d6 "users",
    UserPlus = 0xe4d0 "user-plus",
    Question = 0xe3e8 "question",
    Palette = 0xe6c8 "palette",
    Clock = 0xe19a "clock",
    PushPin = 0xe3e2 "push-pin",
    WarningCircle = 0xe4e2 "warning-circle",
    DotsThreeVertical = 0xe208 "dots-three-vertical",
    DotsSixVertical = 0xeae2 "dots-six-vertical",
    ArrowClockwise = 0xe036 "arrow-clockwise",
    ArrowCounterClockwise = 0xe038 "arrow-counter-clockwise",
    FlipHorizontal = 0xed6a "flip-horizontal",
    ArrowUUpLeft = 0xe08a "arrow-u-up-left",
    Eye = 0xe220 "eye",
    EyeSlash = 0xe224 "eye-slash",
    Star = 0xe46a "star",
    Sticker = 0xe5ac "sticker",
    Trash = 0xe4a6 "trash",
    ArrowUp = 0xe08e "arrow-up",
    FolderSimple = 0xe25a "folder-simple",
    Bell = 0xe0ce "bell",
}

static REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Phosphor-subset.ttf");
static FILL: &[u8] = include_bytes!("../../../assets/fonts/Phosphor-Fill-subset.ttf");

fn family(fill: bool) -> FontFamily {
    FontFamily::Name(if fill { "phosphor-fill" } else { "phosphor" }.into())
}

/// Registers both weights; call once at start (same mechanism as the emoji font).
pub fn register(ctx: &egui::Context) {
    for (name, bytes, fill) in [("phosphor", REGULAR, false), ("phosphor-fill", FILL, true)] {
        ctx.add_font(egui::epaint::text::FontInsert::new(
            name,
            egui::FontData::from_static(bytes),
            vec![egui::epaint::text::InsertFontFamily { family: family(fill), priority: egui::epaint::text::FontPriority::Highest }],
        ));
    }
}

/// The icon of a message delivery state (clock, check, alert) shown next to the time of a bubble.
pub fn stamp_icon(mark: matrix_desktop::StampMark) -> Icon {
    match mark {
        matrix_desktop::StampMark::Pending => Icon::Clock,
        matrix_desktop::StampMark::Sent => Icon::Check,
        matrix_desktop::StampMark::Failed => Icon::WarningCircle,
    }
}

/// The icon as text (for labels and buttons); `fill` is the active/selected weight.
pub fn rich(icon: Icon, fill: bool, size: f32) -> RichText {
    RichText::new(icon.ch()).font(FontId::new(size, family(fill)))
}

/// Icon then text in one button label (text at the body size, icon at the scale's icon size).
pub fn with_label(icon: Icon, text: &str) -> egui::text::LayoutJob {
    let m = crate::metrics::get();
    let mut job = egui::text::LayoutJob::default();
    if crate::dir::rtl() {
        // The line runs from the right whatever the script of the text: the icon stays before it.
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
    }
    job.append(&icon.ch().to_string(), 0., egui::TextFormat { font_id: FontId::new(m.icon, family(false)), color: Color32::PLACEHOLDER, valign: egui::Align::Center, ..Default::default() });
    job.append(text, m.gap, egui::TextFormat { font_id: FontId::proportional(m.body), color: Color32::PLACEHOLDER, valign: egui::Align::Center, ..Default::default() });
    job
}

/// Paints the icon centred on `c` (for custom-drawn cells such as the rail).
/// Icons that point along the reading direction (back, chevron, send, reply, sign out) are mirrored in RTL.
pub fn paint(p: &egui::Painter, c: Pos2, icon: Icon, fill: bool, size: f32, color: Color32) {
    let font = FontId::new(size, family(fill));
    if !(crate::dir::rtl() && matches!(icon, Icon::CaretLeft | Icon::CaretRight | Icon::PaperPlaneRight | Icon::ArrowUUpLeft | Icon::SignOut)) {
        p.text(c, egui::Align2::CENTER_CENTER, icon.ch(), font, color);
        return;
    }
    // The subset fonts hold one direction only: tessellate the glyph and flip its mesh around `c`.
    let galley = p.layout_no_wrap(icon.ch().to_string(), font, color);
    let shape = egui::Shape::galley(egui::Align2::CENTER_CENTER.anchor_size(c, galley.size()).min, galley, color);
    for part in p.ctx().tessellate(vec![egui::epaint::ClippedShape::new(Rect::EVERYTHING, shape)], p.ctx().pixels_per_point()) {
        if let egui::epaint::Primitive::Mesh(mut mesh) = part.primitive {
            for v in &mut mesh.vertices {
                v.pos.x = 2. * c.x - v.pos.x;
            }
            p.add(egui::Shape::mesh(mesh));
        }
    }
}

/// An icon button in `rect`: hover fill like the other buttons, `label` is its screen-reader name. `alpha` cross-fades, `active` false = inert.
pub fn button_at(ui: &mut Ui, rect: Rect, id: egui::Id, icon: Icon, fill: bool, size: f32, label: &str, color: Option<Color32>, active: bool, alpha: f32) -> egui::Response {
    let resp = ui.interact(rect, id, if active { egui::Sense::click() } else { egui::Sense::hover() });
    let v = ui.visuals();
    let hot = active && (resp.hovered() || resp.has_focus());
    let base = color.unwrap_or(v.widgets.inactive.fg_stroke.color);
    let ink = if hot { v.strong_text_color() } else { base }.gamma_multiply(alpha);
    if hot {
        ui.painter().rect_filled(rect, 6., v.widgets.hovered.weak_bg_fill.gamma_multiply(alpha));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, 6., v.selection.stroke, egui::StrokeKind::Inside);
    }
    paint(ui.painter(), rect.center(), icon, fill, size, ink);
    if active {
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// The standard icon-only button: a square of the scale's button height (44 on touch), the scale's icon size.
pub fn button(ui: &mut Ui, icon: Icon, fill: bool, label: &str) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(crate::metrics::button()), egui::Sense::hover());
    button_at(ui, rect, ui.scope_id().with((label, icon.ch())), icon, fill, crate::metrics::get().icon, label, None, true, 1.)
}

/// A context with the icon fonts, for tests that lay out a screen with icons.
#[cfg(test)]
pub fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    register(&ctx);
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Code points of the BMP format-4 cmap of a TrueType font (what pyftsubset writes for these fonts).
    fn cmap(font: &[u8]) -> Vec<u16> {
        let u16at = |o: usize| u16::from_be_bytes([font[o], font[o + 1]]) as usize;
        let u32at = |o: usize| u32::from_be_bytes([font[o], font[o + 1], font[o + 2], font[o + 3]]) as usize;
        let table = (0..u16at(4)).map(|i| 12 + i * 16).find(|&r| &font[r..r + 4] == b"cmap").map(|r| u32at(r + 8)).expect("cmap table");
        let sub = (0..u16at(table + 2)).map(|i| table + u32at(table + 4 + i * 8 + 4)).find(|&s| u16at(s) == 4).expect("format 4 subtable");
        let segs = u16at(sub + 6) / 2;
        let (ends, starts) = (sub + 14, sub + 16 + segs * 2);
        let mut out = Vec::new();
        for i in 0..segs {
            let (end, start) = (u16at(ends + i * 2), u16at(starts + i * 2));
            out.extend((start..=end).filter(|&c| c != 0xffff).map(|c| c as u16));
        }
        out
    }

    #[test]
    fn every_icon_is_in_both_subset_fonts_and_codepoints_are_unique() {
        let (regular, fill) = (cmap(REGULAR), cmap(FILL));
        let mut seen = std::collections::HashSet::new();
        for icon in ALL {
            let cp = icon.ch() as u32;
            assert!(seen.insert(cp), "{icon:?} repeats code point {cp:x}");
            assert!(regular.contains(&(cp as u16)), "{icon:?} missing in Phosphor-subset.ttf");
            assert!(fill.contains(&(cp as u16)), "{icon:?} missing in Phosphor-Fill-subset.ttf");
        }
        assert_eq!(regular.len(), ALL.len(), "the font keeps glyphs no icon uses; rerun the phosphor script");
    }
}
