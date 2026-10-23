use std::collections::HashSet;

use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, RoomId, RoomScope, TimelineId};

#[test]
fn history_retains_every_scoped_event_for_both_workloads() {
    let scope = RoomScope {
        account_id: AccountId::new(0),
        room_id: RoomId::new("!history:fixture.invalid"),
    };
    for count in [2_000, 10_000] {
        let history = DemoHistory::new(scope.clone(), count);
        assert_eq!(history.events().len(), count);
        let ids: HashSet<_> = history.events().iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), count);
        for (index, event) in history.events().iter().enumerate() {
            assert_eq!(event.id.scope, scope);
            assert!(matches!(event.id.id, TimelineId::Server(_)));
            assert!(event.text.starts_with(&format!("Evento {:05}", index + 1)));
            assert_eq!(event.outgoing, index % 3 == 1);
        }
        assert!(history.events()[0].text.contains("fixture offline"));
        assert!(history.events().iter().any(|e| e.text.contains('\n')));
        assert!(history.events().iter().any(|e| e.text.contains("caffè")));
        assert!(history.events().iter().any(|e| e.text.len() > 400));
        let again = DemoHistory::new(scope.clone(), count);
        assert_eq!(history.events(), again.events());
        let other = DemoHistory::new(
            RoomScope {
                account_id: AccountId::new(1),
                room_id: scope.room_id.clone(),
            },
            count,
        );
        assert_ne!(history.events()[0].id, other.events()[0].id);
    }
}
