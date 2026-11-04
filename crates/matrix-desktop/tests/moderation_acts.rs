use matrix_desktop::{ProfileAct as A, confirm_question, moderation_acts};
use matrix_engine::operations::MemberRights;

#[test]
fn plain_user_gets_nothing_moderator_gets_what_the_rights_say_destructive_last() {
    assert_eq!(moderation_acts(&MemberRights::default()), vec![]);
    // Messages count only when there are some to delete.
    let r = MemberRights { kick: true, ban: true, levels: vec![0, 50], recent: vec![] };
    assert_eq!(moderation_acts(&r), vec![A::Power(0), A::Power(50), A::Kick, A::Ban]);
    let r = MemberRights { recent: vec!["$a".into()], ..r };
    let acts = moderation_acts(&r);
    assert_eq!(acts, vec![A::Power(0), A::Power(50), A::RemoveMessages, A::Kick, A::Ban]);
    // Sections never go back: the destructive blocks come after the plain ones.
    assert!(acts.windows(2).all(|w| w[0].section() <= w[1].section()));
    assert!(acts.iter().filter(|a| a.destructive()).all(|a| a.section() >= 2));
    // Only ban allowed (no kick right): no Espelli.
    let r = MemberRights { ban: true, ..Default::default() };
    assert_eq!(moderation_acts(&r), vec![A::Ban]);
}

#[test]
fn confirmation_names_the_user_and_counts_messages() {
    assert_eq!(confirm_question(A::Kick, "Ada", 0), "Espellere Ada?");
    assert_eq!(confirm_question(A::Ban, "Ada", 0), "Bannare Ada?");
    assert_eq!(confirm_question(A::RemoveMessages, "Ada", 7), "Rimuovere 7 messaggi di Ada?");
}

#[test]
fn ignore_is_destructive_and_confirmed_with_the_name_unignore_is_not_and_both_sit_after_moderation() {
    assert_eq!(confirm_question(A::Ignore, "Ada", 0), "Ignorare Ada?");
    assert_eq!((A::Ignore.label(), A::Unignore.label()), ("Ignora", "Smetti di ignorare"));
    assert!(A::Ignore.destructive() && !A::Unignore.destructive());
    for a in [A::Ignore, A::Unignore] {
        assert!(a.section() > A::Ban.section() && a.section() < A::Confirm.section());
    }
}
