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

pub enum Either3<A, B, C> {
    A(A),
    B(B),
    C(C),
}

pin_project! {
    pub struct Select3<'p, A, B, C>
        where A: Producer<'p>,
              B: Producer<'p>,
              C: Producer<'p>
    {
        #[pin]
        a: SelectedProducer<'p, A>,
        #[pin]
        b: SelectedProducer<'p, B>,
        #[pin]
        c: SelectedProducer<'p, C>,
        inner: Arc<Inner>,
        polled_yet: bool,
    }
}

impl<'p, A, B, C> Select3<'p, A, B, C>
    where A: Producer<'p>,
          B: Producer<'p>,
          C: Producer<'p>
{
    pub fn new(a: &'p A, b: &'p B, c: &'p C) -> Self {
        let wake_states = vec![
            AtomicBool::new(false),
            AtomicBool::new(false),
            AtomicBool::new(false),
        ];
        let inner = Arc::new(Inner {
            wake_states,
            task_waker: RwLock::new(None),
        });

        Select3 {
            a: SelectedProducer::new(a, inner.clone(), 0),
            b: SelectedProducer::new(b, inner.clone(), 1),
            c: SelectedProducer::new(c, inner.clone(), 2),
            inner,
            polled_yet: false,
        }
    }

    fn try_take_backlog(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Either3<A::Output, B::Output, C::Output>> {
        let this = self.project();

        if this.a.has_backlog() {
            if this.b.has_backlog() || this.c.has_backlog() {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Either3::A(this.a.take_backlog_and_poll()));
        }

        if this.b.has_backlog() {
            if this.c.has_backlog() {
                cx.waker().wake_by_ref();
            }

            return Poll::Ready(Either3::B(this.b.take_backlog_and_poll()));
        }

        if this.b.has_backlog() {
            return Poll::Ready(Either3::C(this.c.take_backlog_and_poll()));
        }

        Poll::Pending
    }

    pub fn next(self: Pin<&mut Self>) -> Next3<'_, 'p, A, B, C> {
        Next3(self)
    }
}

pub struct Next3<'a, 'p, A, B, C>(Pin<&'a mut Select3<'p, A, B, C>>)
    where A: Producer<'p>,
          B: Producer<'p>,
          C: Producer<'p>;

impl<'p, A, B, C> Future for Next3<'_, 'p, A, B, C>
    where A: Producer<'p>,
          B: Producer<'p>,
          C: Producer<'p>
{
    type Output = Either3<A::Output, B::Output, C::Output>;

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
            this.c.poll_and_store();
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

        if this.inner.wake_states[2].load(Ordering::Acquire) {
            this.c.poll_and_store();
        }

        let this = self.0.as_mut().project();

        let opt = this.a.try_take_backlog_and_poll().map(Either3::A)
            .or(this.b.try_take_backlog_and_poll().map(Either3::B))
            .or(this.c.try_take_backlog_and_poll().map(Either3::C));
        into_poll(opt)
    }
}
