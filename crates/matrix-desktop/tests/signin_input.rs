use matrix_desktop::SignInForm;

#[test]
fn new_signin_prefills_https() {
    assert_eq!(SignInForm::default().homeserver, "https://");
    assert!(!SignInForm::server_entered("https://"));
    assert!(!SignInForm::server_entered(" HTTPS://  "));
    assert!(!SignInForm::server_entered("  "));
    assert!(SignInForm::server_entered(" https://example.org "));
}

#[test]
fn signin_labels_are_translated() {
    use matrix_desktop::i18n::tr;
    assert_eq!(tr("en", "", "Username or Matrix ID"), "Username or Matrix ID");
    assert_eq!(tr("it", "", "Username or Matrix ID"), "Nome utente o ID Matrix");
    assert_eq!(tr("it", "", "Enter a valid username or Matrix ID."), "Inserisci un nome utente o un ID Matrix valido.");
}
