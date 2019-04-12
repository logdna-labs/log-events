quick_error! {
     #[derive(Debug)]
     pub enum WatchError {
        Excluded(err: std::string::String) {
             from()
        }
        InvalidPath(err: std::string::String) {
        }
     }
}

quick_error! {
     #[derive(Debug)]
     pub enum StreamError {
         Blocking(err: tokio_threadpool::BlockingError) {
            from()
         }
         Receive(err: std::sync::mpsc::RecvError) {
             from()
         }
     }
}
