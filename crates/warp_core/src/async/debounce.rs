use std::time::Duration;
use std::{pin, task};

use futures_lite::{Stream, ready};
use pin::Pin;
use pin_project::pin_project;
use task::{Context, Poll};
use warpui_core::r#async::Timer;

/// Debounce takes in a stream and limits the rate of firing events from the stream
/// by bundling all events occurred within the set interval into one.
///
/// For example, if the interval is set to 50 and the following events were fired:
/// E1 fired at T_0
/// E2 fired at T_30
/// E3 fired at T_60
/// E4 fired at T_200
///
/// Debounce will first receive E1 at T_0 and set the timer to expire at T_50. At T_30,
/// E2 came in and the previous timer has not expired, debounce will change the last
/// event to E2 and set timer at T_80 (T_30 + 50). At T_60, E3 came in and similarly
/// the previous timer has not expired, debounce will change the last
/// event to E3 and set timer at T_110 (T_60 + 50). At T_110, the timer expired and emits
/// E3. At T_200, E4 came in and debounce set the timer at T_250. At T_250, E4 was emitted.
///
/// +---------+                      +-----------+    +-----------+
/// | stream  |                      | debounce  |    | executor  |
/// +---------+                      +-----------+    +-----------+
///      |                                 |                |
///      | T_0 E1                          |                |
///      |-------------------------------->|                |
///      |-------------------------------\ |                |
///      || record event and start timer |-|                |
///      ||------------------------------| |                |
///      |                                 |                |
///      | T_30 E2                         |                |
///      |-------------------------------->|                |
///      |  -----------------------------\ |                |
///      |  | timer not expired yet:     |-|                |
///      |  | reset timer and last event | |                |
///      |  |----------------------------| |                |
///      | T_60 E3                         |                |
///      |-------------------------------->|                |
///      |  -----------------------------\ |                |
///      |  | timer not expired yet:     |-|                |
///      |  | reset timer and last event | |                |
///      |  |----------------------------| |                |
///      |                                 | T_110 E3       |
///      |                                 |--------------->|
///      |    ---------------------------\ |                |
///      |    | time expired: emit event |-|                |
///      |    |--------------------------| |                |
///      |                                 |                |
///      | T_200 E4                        |                |
///      |-------------------------------->|                |
///      |    ---------------------------\ |                |
///      |    | time expired: emit event |-|                |
///      |    |--------------------------| |                |
///      |                                 |                |
///      |                                 | T_250 E4       |
///      |                                 |--------------->|
///      |                                 |                |
#[pin_project]
pub struct Debounce<S: Stream> {
    period: Duration,
    #[pin]
    stream: S,
    #[pin]
    timer: Option<Timer>,
    last_item: Option<S::Item>,
    has_expired: bool,
}

pub fn debounce<S>(period: Duration, stream: S) -> impl Stream<Item = S::Item>
where
    S: Stream,
{
    Debounce {
        period,
        stream,
        timer: None,
        last_item: None,
        has_expired: false,
    }
}

impl<S> Stream for Debounce<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        // Stream has expired already--return Poll::Ready(None).
        if *this.has_expired {
            return Poll::Ready(None);
        }

        let mut stream = this.stream;

        // Read out everything from the stream until the stream is exhausted.
        while let Poll::Ready(item) = stream.as_mut().poll_next(ctx) {
            match item {
                Some(item) => {
                    *this.last_item = Some(item);
                    *this.timer = Some(Timer::after(*this.period));
                }
                None => {
                    *this.timer = None;
                    *this.has_expired = true;
                    return Poll::Ready(this.last_item.take());
                }
            }
        }

        if let Some(timer) = this.timer.as_pin_mut() {
            ready!(timer.poll_next(ctx));

            // The timer is done--return the last item.
            let mut this = self.project();
            *this.timer = None;
            return Poll::Ready(this.last_item.take());
        }

        Poll::Pending
    }
}
