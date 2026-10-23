use matrix_desktop::{ScrollbarGeometry, VirtualScrollExtent};

#[test]
fn unmeasured_history_remains_reachable_without_laying_out_all_rows() {
    let mut extent = VirtualScrollExtent::new(2000);
    extent.measure(1998, 100.);
    extent.measure(1999, 200.);
    assert_eq!(extent.total_height(), 300_000.);
    assert_eq!(extent.offset_for_item(1999, 0.), 299_800.);
    assert_eq!(extent.item_for_offset(0.), (0, 0.));
    assert_eq!(extent.item_for_offset(299_850.), (1999, 50.));
    assert_eq!(extent.item_for_offset(300_000.), (2000, 0.));
}

#[test]
fn fitting_or_invalid_extents_have_no_interactive_thumb() {
    for (viewport, content, offset) in [
        (600., 600., 0.),
        (600., 0., 0.),
        (0., 200., 0.),
        (f32::NAN, 200., 0.),
        (600., f32::INFINITY, 0.),
        (600., 2400., f32::NAN),
    ] {
        assert!(ScrollbarGeometry::new(viewport, content, offset).is_none());
    }
}

#[test]
fn minimum_thumb_keeps_drag_mapping_clamped() {
    let g = ScrollbarGeometry::new(600., 1_000_000., 0.).unwrap();
    assert_eq!(g.thumb_length, 28.);
    assert_eq!(g.offset_for_thumb(-50.), 0.);
    assert_eq!(g.offset_for_thumb(1000.), 999_400.);
}

#[test]
fn tiny_viewport_still_has_finite_drag_travel() {
    let g = ScrollbarGeometry::new(20., 100., 0.).unwrap();
    assert!(g.offset_for_thumb(20.).is_finite());
    assert_eq!(g.offset_for_thumb(20.), 80.);
}

#[test]
fn thumb_tracks_viewport_fraction_and_both_endpoints() {
    let top = ScrollbarGeometry::new(600., 2400., 0.).unwrap();
    assert_eq!(top.thumb_length, 150.);
    assert_eq!(top.thumb_top, 0.);
    let bottom = ScrollbarGeometry::new(600., 2400., 1800.).unwrap();
    assert_eq!(bottom.thumb_top, 450.);
    assert_eq!(bottom.offset_for_thumb(450.), 1800.);
}
