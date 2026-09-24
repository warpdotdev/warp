use std::borrow::Cow;

use pathfinder_geometry::vector::vec2f;
use warpui::units::{IntoPixels as _, Pixels};

use super::*;

#[test]
fn replacement_sender_queues_resize_and_input_while_suspended() {
    let (initial_tx, _initial_rx) = mio_channel::channel();
    let sender = ReplaceableEventLoopSender::new(initial_tx);
    sender.suspend();

    let size = SizeInfo::new(
        vec2f(80., 24.),
        1.0.into_pixels(),
        1.0.into_pixels(),
        Pixels::zero(),
        Pixels::zero(),
    );
    sender
        .send(Message::Resize(size))
        .expect("resize should queue while recovery is suspended");
    sender
        .send(Message::Input(Cow::Borrowed(b"user input")))
        .expect("input should queue while recovery is suspended");

    let (replacement_tx, replacement_rx) = mio_channel::channel();
    sender.set_bootstrap_sender(replacement_tx);
    sender
        .send_bootstrap(Message::Input(Cow::Borrowed(b"bootstrap")))
        .expect("bootstrap input should reach the provisional event loop");
    assert!(matches!(
        replacement_rx.try_recv(),
        Ok(Message::Input(bytes)) if &*bytes == b"bootstrap"
    ));
    assert!(replacement_rx.try_recv().is_err());

    sender.resume().expect("replacement sender should resume");
    assert!(matches!(replacement_rx.try_recv(), Ok(Message::Resize(_))));
    assert!(matches!(
        replacement_rx.try_recv(),
        Ok(Message::Input(bytes)) if &*bytes == b"user input"
    ));
    assert!(replacement_rx.try_recv().is_err());
}
