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
    FilterAll,
    Star,
    LowPriority,
    Folder,
    Dot,
    Ring,
    People,
    Shield,
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
        Icon::FilterAll => vec![
            vec![(8., 5.), (16., 5.)],
            vec![(8., 10.), (16., 10.)],
            vec![(8., 15.), (16., 15.)],
            arc((4.5, 5.), 0.8, 0., 2. * PI), // ring thinner than the stroke = solid dot
            arc((4.5, 10.), 0.8, 0., 2. * PI), // ring thinner than the stroke = solid dot
            arc((4.5, 15.), 0.8, 0., 2. * PI), // ring thinner than the stroke = solid dot
        ],
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let (r, a) = (if i % 2 == 0 { 7.5 } else { 3.2 }, -PI / 2. + i as f32 * PI / 5.);
                    (10. + r * a.cos(), 10.5 + r * a.sin())
                })
                .collect();
            vec![pts]
        }
        Icon::LowPriority => vec![
            vec![(10., 3.), (10., 13.)],
            vec![(5.5, 8.5), (10., 13.), (14.5, 8.5)],
            vec![(4., 17.), (16., 17.)],
        ],
        Icon::Folder => vec![vec![(3., 6.), (8., 6.), (10., 8.), (17., 8.), (17., 16.), (3., 16.), (3., 6.)]],
        // Strokes only: a filled dot is concentric rings one stroke apart.
        Icon::Dot => (1..=4).map(|r| arc((10., 10.), r as f32, 0., 2. * PI)).collect(),
        Icon::People => vec![
            arc((10., 7.), 3., 0., 2. * PI),
            arc((10., 17.), 6., PI, 2. * PI),
        ],
        Icon::Shield => vec![vec![(10., 2.5), (16., 5.), (16., 10.), (10., 17.5), (4., 10.), (4., 5.), (10., 2.5)]],
        Icon::Ring => vec![arc((10., 10.), 5.5, 0., 2. * PI)],
        Icon::Check => vec![vec![(4., 10.5), (8.5, 15.), (16., 5.5)]],
        Icon::Emoji => vec![
            arc((10., 10.), 7.5, 0., 2. * PI),
            vec![(7.5, 8.), (7.5, 9.)],
            vec![(12.5, 8.), (12.5, 9.)],
            arc((10., 10.), 4., 0.25 * PI, 0.75 * PI),
        ],
    }
}

pub fn icon(kind: Icon, color: u32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for line in strokes(kind) {
                let mut path = PathBuilder::stroke(px(1.6));
                for (i, (x, y)) in line.into_iter().enumerate() {
                    let p = point(bounds.left() + px(x * 1.2), bounds.top() + px(y * 1.2));
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
    .w(px(24.))
    .h(px(24.))
}
