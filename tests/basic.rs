#![cfg(not(loom))]

use umthombo::{select2::{Select2, Either2}, select3::{Select3, Either3}, Producer};
use std::future::Future;
use std::task::{Poll, Context};
use std::pin::Pin;

#[tokio::test]
async fn select_ready_always_first() {
    let selector = Select2::new(&AlwaysReady, &AlwaysReady);
    futures::pin_mut!(selector);
    let next = selector.next().await;
    assert!(matches!(Either2::<(), ()>::A(()), next));
}

#[tokio::test]
async fn select_pending() {
    let selector = Select2::new(&AlwaysPending, &AlwaysReady);
    futures::pin_mut!(selector);
    let next = selector.next().await;
    assert!(matches!(Either2::<(), ()>::A(()), next));
}

#[tokio::test]
async fn select3_pending() {
    let selector = Select3::new(&AlwaysPending, &AlwaysReady, &AlwaysPending);
    futures::pin_mut!(selector);
    let next = selector.next().await;
    assert!(matches!(Either3::<(), (), ()>::B(()), next));
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
