# umthombo

Select over two or three "streams" of futures, only polling whichever is woken.

## Motivation

1. Why not just use `Stream` select? This is because while `Future`s have a `drop`, `Stream`s do not have a "`poll_cancel`".
This means that for channels which have stream impls, they have to *always notify every stream*, lest one of them has
since stopped polling and released the "resource" that is a message. This can make selecting over `Stream`s undesireable.
This is one of the niches that umthombo fills, until such a time as streams get a `poll_cancel` method or similar,
or indefinitely.

2. The second reason for umthombo's existence is to lower the overhead of selecting when both futures should not be
eagerly polled, but one is still more important than another. Umthombo will only poll woken futures, but it will also
prefer to poll the futures in order.
