#[cfg(not(loom))]
use spinny::RwLock;
#[cfg(loom)]
use loom::sync::RwLock;

use crate::{Producer, Inner, SelectedProducer, into_poll};
use crate::sync::atomic::{AtomicBool, Ordering};
use crate::sync::Arc;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::future::Future;
use pin_project_lite::pin_project;

pub enum Either2<A, B> {
    A(A),
    B(B)
}

pin_project! {
    pub struct Select2<'p, A, B>
        where A: Producer<'p>,
              B: Producer<'p>
    {
        #[pin]
        a: SelectedProducer<'p, A>,
        #[pin]
        b: SelectedProducer<'p, B>,
        inner: Arc<Inner>,
        polled_yet: bool,
    }
}

impl<'p, A, B> Select2<'p, A, B>
    where A: Producer<'p>,
          B: Producer<'p>
{
    pub fn new(a: &'p A, b: &'p B) -> Self {
        let inner = Arc::new(Inner {
            wake_states: vec![AtomicBool::new(false), AtomicBool::new(false)],
            task_waker: RwLock::new(None),
        });

        Select2 {
            a: SelectedProducer::new(a, inner.clone(), 0),
            b: SelectedProducer::new(b, inner.clone(), 1),
            inner,
            polled_yet: false,
        }
    }

    fn try_take_backlog(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Either2<A::Output, B::Output>> {
        let this = self.project();

        if this.a.has_backlog() {
            if this.b.has_backlog() {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Either2::A(this.a.take_backlog_and_poll()));
        }

        if this.b.has_backlog() {
            return Poll::Ready(Either2::B(this.b.take_backlog_and_poll()));
        }

        Poll::Pending
    }

    pub fn next(self: Pin<&mut Self>) -> Next2<'_, 'p, A, B> {
        Next2(self)
    }
}

pub struct Next2<'a, 'p, A, B>(Pin<&'a mut Select2<'p, A, B>>)
    where A: Producer<'p>,
          B: Producer<'p>;

impl<'p, A, B> Future for Next2<'_, 'p, A, B>
    where A: Producer<'p>,
          B: Producer<'p>
{
    type Output = Either2<A::Output, B::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            #[cfg(not(loom))]
                let mut waker = self.0.inner.task_waker.write();
            #[cfg(loom)]
                let mut waker = self.0.inner.task_waker.write().unwrap();

            *waker = Some(cx.waker().clone())
        }

        // First poll - set up futures by polling each
        if !self.0.polled_yet {
            let this = self.0.as_mut().project();
            this.a.poll_and_store();
            this.b.poll_and_store();
            // Take the output from one of them. A first, else B.
            return self.0.as_mut().try_take_backlog(cx);
        }

        // Get rid of any pre-existing backlog, and then poll the future again, storing that to
        // backlog. This *can* enter a loop of backlog clearing, but at least it is fair - first
        // A will go, then B, then A, etc... This also means that they are immediately ready.
        if let Poll::Ready(out) = self.0.as_mut().try_take_backlog(cx) {
            return Poll::Ready(out);
        }

        // Poll whichever futures need to be polled
        let this = self.0.as_mut().project();
        if this.inner.wake_states[0].load(Ordering::Acquire) {
            this.a.poll_and_store();
        }

        if this.inner.wake_states[1].load(Ordering::Acquire) {
            this.b.poll_and_store();
        }

        let this = self.0.as_mut().project();
        let opt = this.a.try_take_backlog_and_poll().map(Either2::A)
            .or(this.b.try_take_backlog_and_poll().map(Either2::B));
        into_poll(opt)
    }
}
