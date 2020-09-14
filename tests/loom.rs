#![cfg(loom)]

use tokio::sync::mpsc::{self, Receiver};
use umthombo::Select;
use umthombo::Producer;
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use loom::thread;
use loom::future::block_on;
use loom::sync::{Arc, Mutex};

#[test]
fn select_wakeups_propagate() {
    loom::model(|| {
        let (mut tx, rx) = mpsc::channel(16);

        let th1 = thread::spawn(move || {
            let prod = ChannelProducer(Arc::new(Mutex::new(rx)));
            let selector = Select::new(&prod, &AlwaysPending);
            futures::pin_mut!(selector);
            block_on(selector.next());
        });

        let th2 = thread::spawn(move || {
            block_on(tx.send(())).unwrap();
        });

        th1.join().unwrap();
        th2.join().unwrap();
    });
}

struct AlwaysPending;

impl<'a> Producer<'a> for AlwaysPending {
    type Output = ();
    type Future = futures::future::Pending<()>;

    fn produce(&'a self) -> Self::Future {
        futures::future::pending()
    }
}

struct ChannelProducer(Arc<Mutex<Receiver<()>>>);

impl<'a> Producer<'a> for ChannelProducer {
    type Output = Option<()>;
    type Future = RecvFut;

    fn produce(&'a self) -> Self::Future {
        RecvFut(self.0.clone())
    }
}

struct RecvFut(Arc<Mutex<Receiver<()>>>);

impl Future for RecvFut {
    type Output = Option<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().0.lock().unwrap().poll_recv(cx)
    }
}
