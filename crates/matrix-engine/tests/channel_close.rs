use matrix_engine::{TryReceiveError, TrySendError, bounded};
use std::num::NonZeroUsize;

#[test]
fn closing_a_receiver_rejects_new_commands_but_drains_accepted_commands() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(2).unwrap());
    assert_eq!(sender.try_send(22), Ok(()));
    receiver.close();
    assert_eq!(sender.try_send(23), Err(TrySendError::Closed(23)));
    assert_eq!(receiver.try_recv(), Ok(22));
    assert_eq!(receiver.try_recv(), Err(TryReceiveError::Closed));
    receiver.close();
}
