use matrix_desktop::{VerifyMark, verify_mark};

#[test]
fn verification_item_follows_identity_and_self() {
    // Oneself: nothing, whatever the identity says.
    assert_eq!(verify_mark(true, Some(false)), VerifyMark::Hidden);
    assert_eq!(verify_mark(true, Some(true)), VerifyMark::Hidden);
    assert_eq!(verify_mark(true, None), VerifyMark::Hidden);
    // Another user: offer while unverified, the sign once verified, nothing without an identity.
    assert_eq!(verify_mark(false, Some(false)), VerifyMark::Offer);
    assert_eq!(verify_mark(false, Some(true)), VerifyMark::Verified);
    assert_eq!(verify_mark(false, None), VerifyMark::Hidden);
}
