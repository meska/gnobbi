use matrix_domain::AccountId;

#[test]
fn account_id_preserves_local_identity_value() {
    for value in [0, 1, 42, u64::MAX] {
        let id = AccountId::new(value);
        assert_eq!(id.as_u64(), value);
    }
}
