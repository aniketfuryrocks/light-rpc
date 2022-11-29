use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Future;

pub struct LightBridge;

impl Future for LightBridge {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(())
    }
}
