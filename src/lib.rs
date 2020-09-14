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

pub mod select2;
pub mod select3;

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

    fn try_take_backlog_and_poll(self: Pin<&mut Self>) -> Option<P::Output> {
        if self.has_backlog() {
            Some(self.take_backlog_and_poll())
        } else {
            None
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
    wake_states: Vec<AtomicBool>,
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

pub trait Producer<'a> {
    type Output;
    type Future: Future<Output = Self::Output> + 'a;
    fn produce(&'a self) -> Self::Future;
}

fn into_poll<T>(opt: Option<T>) -> Poll<T> {
    opt.map(Poll::Ready).unwrap_or(Poll::Pending)
}
