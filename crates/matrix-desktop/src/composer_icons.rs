//! Composer button icons, drawn as stroked paths (no asset files). Meaning is carried by the a11y tree, not by text.
use gpui::{IntoElement, PathBuilder, canvas, point, prelude::*, px, rgb};
use std::f32::consts::PI;

#[derive(Clone, Copy)]
pub enum Icon {
    Attach,
    Voice,
    Emoji,
    Send,
    Back,
    Close,
    Copy,
    Check,
    Search,
    Poll,
    /// Pin (head and needle): «Fissa in alto».
    Pin,
    /// Pin with a solid head: the room is pinned.
    PinFilled,
    LowPriority,
    People,
    Shield,
    Play,
    Pause,
    Download,
    Ellipsis,
    /// Speech bubble: «Tutte le chat».
    Chats,
    /// Speech bubble with a dot: «Non lette».
    Unread,
    Alert,
    /// Bell: this room's notification mode; BellOff (struck through) is a muted room.
    Bell,
    BellOff,
}

fn arc(c: (f32, f32), r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    (0..=16)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / 16.;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// Polylines on a 20x20 grid, painted at 1.2x (24 px).
fn strokes(icon: Icon) -> Vec<Vec<(f32, f32)>> {
    match icon {
        Icon::Attach => {
            let mut clip = vec![(9., 12.), (9., 6.)];
            clip.extend(arc((11., 6.), 2., PI, 2. * PI));
            clip.push((13., 13.));
            clip.extend(arc((10., 13.), 3., 0., PI));
            clip.push((7., 7.));
            vec![clip]
        }
        Icon::Voice => {
            let mut body = arc((10., 5.5), 2.5, PI, 2. * PI);
            body.extend(arc((10., 9.), 2.5, 0., PI));
            body.push((7.5, 5.5));
            vec![
                body,
                arc((10., 9.), 5.5, 0., PI),
                vec![(10., 14.5), (10., 17.5)],
                vec![(7., 17.5), (13., 17.5)],
            ]
        }
        Icon::Send => vec![vec![(10., 16.), (10., 4.)], vec![(4.5, 9.5), (10., 4.), (15.5, 9.5)]],
        Icon::Back => vec![vec![(12.5, 4.), (6.5, 10.), (12.5, 16.)]],
        Icon::Close => vec![vec![(5., 5.), (15., 15.)], vec![(15., 5.), (5., 15.)]],
        Icon::Copy => vec![
            vec![(8., 8.), (15., 8.), (15., 16.), (8., 16.), (8., 8.)],
            vec![(5., 12.), (5., 4.), (12., 4.)],
        ],
        Icon::Search => vec![arc((8.5, 8.5), 5.5, 0., 2. * PI), vec![(12.5, 12.5), (17., 17.)]],
        Icon::Poll => vec![
            vec![(4., 5.), (14., 5.)],
            vec![(4., 10.), (16., 10.)],
            vec![(4., 15.), (10., 15.)],
        ],
        Icon::Pin | Icon::PinFilled => vec![
            arc((10., 6.5), 3.5, 0., 2. * PI),
            vec![(10., 10.), (10., 17.5)],
            vec![(6., 10.), (14., 10.)],
        ],
        Icon::LowPriority => vec![
            vec![(10., 3.), (10., 13.)],
            vec![(5.5, 8.5), (10., 13.), (14.5, 8.5)],
            vec![(4., 17.), (16., 17.)],
        ],
        Icon::People => vec![
            arc((10., 7.), 3., 0., 2. * PI),
            arc((10., 17.), 6., PI, 2. * PI),
        ],
        Icon::Shield => vec![vec![(10., 2.5), (16., 5.), (16., 10.), (10., 17.5), (4., 10.), (4., 5.), (10., 2.5)]],
        Icon::Play => vec![vec![(6., 3.), (6., 17.), (16., 10.), (6., 3.)]],
        Icon::Pause => vec![
            vec![(5.5, 4.), (8.5, 4.), (8.5, 16.), (5.5, 16.), (5.5, 4.)],
            vec![(11.5, 4.), (14.5, 4.), (14.5, 16.), (11.5, 16.), (11.5, 4.)],
        ],
        Icon::Download => vec![vec![(10., 3.), (10., 13.)], vec![(5.5, 8.5), (10., 13.), (14.5, 8.5)], vec![(4., 17.), (16., 17.)]],
        Icon::Ellipsis => [4.5, 10., 15.5].map(|x| arc((x, 10.), 0.8, 0., 2. * PI)).to_vec(),
        Icon::Chats | Icon::Unread => {
            let mut v = vec![vec![(3.5, 4.), (16.5, 4.), (16.5, 13.5), (9.5, 13.5), (5.5, 17.), (5.5, 13.5), (3.5, 13.5), (3.5, 4.)]];
            v.push(if matches!(icon, Icon::Unread) { arc((10., 8.75), 1.4, 0., 2. * PI) } else { vec![(7., 8.75), (13., 8.75)] });
            v
        }
        // Triangle with an exclamation mark: not connected.
        Icon::Alert => vec![vec![(10., 3.), (17.5, 16.5), (2.5, 16.5), (10., 3.)], vec![(10., 8.), (10., 12.)], vec![(10., 14.4), (10., 14.6)]],
        Icon::Bell | Icon::BellOff => {
            let mut bell = vec![(3.5, 14.5), (5., 13.), (5., 8.)];
            bell.extend(arc((10., 8.), 5., PI, 2. * PI));
            bell.extend([(15., 13.), (16.5, 14.5), (3.5, 14.5)]);
            let mut v = vec![bell, vec![(8.5, 17.), (11.5, 17.)]];
            if matches!(icon, Icon::BellOff) {
                v.push(vec![(3., 3.), (17., 17.)]);
            }
            v
        }
        Icon::Check => vec![vec![(4., 10.5), (8.5, 15.), (16., 5.5)]],
        Icon::Emoji => vec![
            arc((10., 10.), 7.5, 0., 2. * PI),
            vec![(7.5, 8.), (7.5, 9.)],
            vec![(12.5, 8.), (12.5, 9.)],
            arc((10., 10.), 4., 0.25 * PI, 0.75 * PI),
        ],
    }
}

/// Grid point that lands on the canvas centre: the bounding-box centre of the strokes, so every
/// glyph is centred in its button whatever the shape. The Play triangle uses its centroid instead
/// (optical centring: the box centre would look too far left of the hit area's centre).
fn anchor(kind: Icon, lines: &[Vec<(f32, f32)>]) -> (f32, f32) {
    if matches!(kind, Icon::Play) {
        let v = &lines[0][..3];
        return (v.iter().map(|p| p.0).sum::<f32>() / 3., v.iter().map(|p| p.1).sum::<f32>() / 3.);
    }
    let pts = || lines.iter().flatten();
    let (min, max) = (|f: fn(&(f32, f32)) -> f32| pts().map(f).fold(f32::MAX, f32::min), |f: fn(&(f32, f32)) -> f32| pts().map(f).fold(f32::MIN, f32::max));
    ((min(|p| p.0) + max(|p| p.0)) / 2., (min(|p| p.1) + max(|p| p.1)) / 2.)
}

pub fn icon(kind: Icon, color: u32) -> impl IntoElement {
    icon_sized(kind, color, 24.)
}

/// Same drawing on a `side` px square (the 20x20 grid scaled), for marks inside a text line.
pub fn icon_sized(kind: Icon, color: u32, side: f32) -> impl IntoElement {
    let k = side / 20.;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let lines = strokes(kind);
            let (ax, ay) = anchor(kind, &lines);
            let (ox, oy) = (bounds.center().x - px(ax * k), bounds.center().y - px(ay * k));
            for line in lines {
                if matches!(kind, Icon::PinFilled | Icon::Play | Icon::Pause) {
                    let mut fill = PathBuilder::fill();
                    for (i, (x, y)) in line.iter().enumerate() {
                        let p = point(ox + px(x * k), oy + px(y * k));
                        if i == 0 { fill.move_to(p) } else { fill.line_to(p) }
                    }
                    fill.close();
                    if let Ok(fill) = fill.build() {
                        window.paint_path(fill, rgb(color));
                    }
                }
                let mut path = PathBuilder::stroke(px(1.6 * k / 1.2));
                for (i, (x, y)) in line.into_iter().enumerate() {
                    let p = point(ox + px(x * k), oy + px(y * k));
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .w(px(side))
    .h(px(side))
    .flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_centred_on_its_anchor() {
        for kind in [Icon::Attach, Icon::Voice, Icon::Emoji, Icon::Send, Icon::Back, Icon::Close, Icon::Copy, Icon::Check, Icon::Search, Icon::Poll, Icon::Pin, Icon::PinFilled, Icon::LowPriority, Icon::People, Icon::Shield, Icon::Pause, Icon::Download, Icon::Ellipsis] {
            let lines = strokes(kind);
            let (ax, ay) = anchor(kind, &lines);
            let pts: Vec<_> = lines.iter().flatten().collect();
            let mid = |f: fn(&&(f32, f32)) -> f32| (pts.iter().map(f).fold(f32::MAX, f32::min) + pts.iter().map(f).fold(f32::MIN, f32::max)) / 2.;
            assert!((ax - mid(|p| p.0)).abs() < 1e-3 && (ay - mid(|p| p.1)).abs() < 1e-3);
        }
        // Play: centroid of the triangle, left of the box centre by ~8% of the width.
        let (ax, ay) = anchor(Icon::Play, &strokes(Icon::Play));
        assert!((ax - (6. + 6. + 16.) / 3.).abs() < 1e-3 && (ay - 10.).abs() < 1e-3);
    }
}
