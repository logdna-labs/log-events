#[macro_use] extern crate log;
#[macro_use] extern crate quick_error;

use std::path::PathBuf;

pub mod rule;
pub mod error;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
pub type RecommendedWatcher = crate::mac::MacOsWatcher;

#[cfg(target_os = "linux")]
pub type RecommendedWatcher = crate::linux::LinuxWatcher;

#[derive(Debug)]
pub enum Event {
    Create(PathBuf),
    Write(PathBuf),
    Delete(PathBuf),
    Init(PathBuf),
}

#[cfg(test)]
mod tests {
    use futures::future;
    use futures::Stream;
    use futures::Future;

    use super::*;

    #[test]
    fn it_works() {
        let mut watcher = RecommendedWatcher::new(crate::rule::Rules::new());
        match watcher.add("/var/log/at/test.txt") {
            Ok(_) => {
            },
            Err(e) => {
                error!("{:?}", &e);
            }
        };
        println!("watching {:?}", &watcher.watched());
        watcher.init();
        tokio::run(watcher
            .stream()
            .for_each(|event|
                future::ok(println!("{:?}", &event))
            ).map_err(|e| error!("{:?}", &e))
        )
    }
}
