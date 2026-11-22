#[path = "../src/platform_impl/ios/image_insertion.rs"]
mod image_insertion;
use image_insertion::*;

#[test]
fn image_queue_is_bounded_and_consumed_once() {
    let mut queue = ImageInsertions::default();
    queue.push(Ok((vec![1, 2], Some("A glyph".into()))));
    assert_eq!(queue.take(), vec![Ok((vec![1, 2], Some("A glyph".into())))]);
    assert!(queue.take().is_empty());
    queue.push(Ok((vec![1], None)));
    queue.push(Ok((vec![2], None)));
    assert_eq!(queue.take(), vec![Err("Only one attachment is supported".into())]);
    queue.push(Ok((vec![1], None)));
    queue.clear();
    assert!(queue.take().is_empty());
}

#[test]
fn encoded_and_decoded_limits_reject_invalid_inputs() {
    let mut queue = ImageInsertions::default();
    queue.push(Ok((vec![0; MAX_BYTES + 1], None)));
    assert_eq!(queue.take(), vec![Err("Image is too large".into())]);
    queue.push(Ok((Vec::new(), None)));
    assert!(queue.take()[0].is_err());
    assert!(valid_dimensions(2000.0, 4000.0));
    for (w, h) in [(0.0, 1.0), (8193.0, 1.0), (4000.0, 4000.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
        assert!(!valid_dimensions(w, h));
    }
}
