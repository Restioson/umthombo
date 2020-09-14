#![cfg(not(loom))]

use umthombo::{Select, Producer, Either};
use std::future::Future;
use std::task::{Poll, Context};
use std::pin::Pin;

#[tokio::test]
async fn select_ready_always_first() {
    let selector = Select::new(&AlwaysReady, &AlwaysReady);
    futures::pin_mut!(selector);
    let next = selector.next().await;
    assert!(matches!(Either::<(), ()>::A(()), next));
}

#[tokio::test]
async fn select_pending() {
    let selector = Select::new(&AlwaysPending, &AlwaysReady);
    futures::pin_mut!(selector);
    let next = selector.next().await;
    assert!(matches!(Either::<(), ()>::A(()), next));
}

struct AlwaysReady;

impl<'a> Producer<'a> for AlwaysReady {
    type Output = ();
    type Future = futures::future::Ready<()>;

    fn produce(&'a self) -> Self::Future {
        futures::future::ready(())
    }
}

struct AlwaysPending;

impl<'a> Producer<'a> for AlwaysPending {
    type Output = ();
    type Future = futures::future::Pending<()>;

    fn produce(&'a self) -> Self::Future {
        futures::future::pending()
    }
}
