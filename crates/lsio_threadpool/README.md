`lsio_threadpool` provides a simple [work stealing](https://en.wikipedia.org/wiki/Work_stealing) threadpool.

`lsio_threadpool` is a fairly minimal wrapper around [`crossbeam_deque`]. The vast bulk of the fiddly, low-level implementation of work stealing is provided by [`crossbeam_deque`]!

To get started, please read the documentation for [`ThreadPool::new`].
