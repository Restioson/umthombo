use std::future::Future;
use std::task::{Context, Poll, Waker};
use std::pin::Pin;
use pin_project_lite::pin_project;

#[cfg(not(loom))]
use spinny::RwLock;
#[cfg(loom)]
use loom::sync::RwLock;

/// IntelliJ IDEA-friendly loom facade
mod sync {
    #[cfg(not(loom))]
    pub use std::sync::*;
    #[cfg(loom)]
    pub use loom::sync::*;

    pub mod atomic {
        #[cfg(not(loom))]
        pub use std::sync::atomic::*;
        #[cfg(loom)]
        pub use loom::sync::atomic::*;
    }
}

use sync::Arc;
use sync::atomic::{AtomicBool, Ordering};

pin_project! {
    pub struct Select<'p, A, B>
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

impl<'p, A, B> Select<'p, A, B>
    where A: Producer<'p>,
          B: Producer<'p>
{
    pub fn new(a: &'p A, b: &'p B) -> Self {
        let inner = Arc::new(Inner {
            wake_states: [AtomicBool::new(false), AtomicBool::new(false)],
            task_waker: RwLock::new(None),
        });

        Select {
            a: SelectedProducer::new(a, inner.clone(), 0),
            b: SelectedProducer::new(b, inner.clone(), 1),
            inner,
            polled_yet: false,
        }
    }

    fn try_take_backlog(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Either<A::Output, B::Output>> {
        let this = self.project();

        if this.a.has_backlog() {
            if this.b.has_backlog() {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Either::A(this.a.take_backlog_and_poll()));
        }

        if this.b.has_backlog() {
            return Poll::Ready(Either::B(this.b.take_backlog_and_poll()));
        }

        Poll::Pending
    }

    pub fn next(self: Pin<&mut Self>) -> Next<'_, 'p, A, B> {
        Next(self)
    }
}

pub struct Next<'a, 'p, A, B>(Pin<&'a mut Select<'p, A, B>>)
    where A: Producer<'p>,
          B: Producer<'p>;

impl<'p, A, B> Future for Next<'_, 'p, A, B>
    where A: Producer<'p>,
          B: Producer<'p>
{
    type Output = Either<A::Output, B::Output>;

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
        match this.a.try_take_backlog_and_poll() {
            Poll::Ready(out) => Poll::Ready(Either::A(out)),
            Poll::Pending => this.b.try_take_backlog_and_poll().map(Either::B),
        }
    }
}

pin_project! {
    struct SelectedProducer<'a, P: Producer<'a>> {
        producer: &'a P,
        #[pin]
        future: P::Future,
        output_backlog: Option<P::Output>,
        waker: Waker,
    }
}

impl<'a, P: Producer<'a>> SelectedProducer<'a, P> {
    fn new(producer: &'a P, inner: Arc<Inner>, idx: u8) -> Self {
        SelectedProducer {
            producer,
            future: producer.produce(),
            output_backlog: None,
            waker: waker_fn::waker_fn(move || {
                inner.wake_states[idx as usize].store(true, Ordering::Release);
                inner.try_wake();
            })
        }
    }

    fn has_backlog(&self) -> bool {
        self.output_backlog.is_some()
    }

    fn try_take_backlog_and_poll(self: Pin<&mut Self>) -> Poll<P::Output> {
        if self.has_backlog() {
            Poll::Ready(self.take_backlog_and_poll())
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn take_backlog_and_poll(mut self: Pin<&mut Self>) -> P::Output {
        let out = {
            let this = self.as_mut().project();
            this.output_backlog.take().unwrap()
        };

        self.poll_and_store();
        out
    }

    fn poll_inner(mut self: Pin<&mut Self>) -> Poll<P::Output> {
        let poll = {
            let this = self.as_mut().project();
            this.future.poll(&mut Context::from_waker(&this.waker))
        };

        match poll {
            Poll::Ready(res) => {
                let mut this = self.project();
                this.future.set(this.producer.produce());
                Poll::Ready(res)
            },
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_and_store(mut self: Pin<&mut Self>) {
        let poll = self.as_mut().poll_inner();
        *self.project().output_backlog = match poll {
            Poll::Ready(res) => Some(res),
            Poll::Pending => None,
        };
    }
}

#[derive(Debug)]
struct Inner {
    wake_states: [AtomicBool; 2],
    task_waker: RwLock<Option<Waker>>,
}

impl Inner {
    fn try_wake(&self) {
        #[cfg(not(loom))]
        let waker = self.task_waker.read();
        #[cfg(loom)]
        let waker = self.task_waker.read().unwrap();

        if let Some(waker) = &*waker {
            waker.wake_by_ref();
        }
    }
}

pub enum Either<A, B> {
    A(A),
    B(B)
}

pub trait Producer<'a> {
    type Output;
    type Future: Future<Output = Self::Output> + 'a;
    fn produce(&'a self) -> Self::Future;
}
