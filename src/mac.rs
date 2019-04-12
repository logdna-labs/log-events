use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::mem;

use crate::Event;
use crate::rule::Rules;
use crate::error::{WatchError, StreamError};

use futures::{Async, Stream, try_ready};
use hashbrown::HashSet;
use tokio_threadpool::blocking;
use glob::glob;
use fsevent;

pub struct MacOsWatcher {
    watched_files: HashSet<PathBuf>,
    watched_dirs: Vec<String>,
    fsevent_sender: Sender<fsevent::Event>,
    fsevent_receiver: Receiver<fsevent::Event>,
    fsevent_thread: Option<thread::JoinHandle<()>>, // Probably should find to do this with tokio. Will do it next time...
    rules: Rules,
}

impl MacOsWatcher {
    pub fn new(rules: Rules) -> MacOsWatcher {
        let (sender, receiver) = channel();

        MacOsWatcher {
            watched_files: HashSet::new(),
            watched_dirs: Vec::new(),
            fsevent_sender: sender,
            fsevent_receiver: receiver,
            fsevent_thread: None,
            rules: rules,
        }
    }

    pub fn stream(self) -> impl Stream<Item=Event, Error=StreamError> {
        MacOsEventStream {
            watcher: self,
            events: Vec::new()
        }
    }

    pub fn watched(&self) -> Vec<PathBuf> {
        let mut watched = Vec::with_capacity(self.watched_files.len());
        for file in &self.watched_files {
            watched.push(file.clone());
        }

        watched
    }

    pub fn add<T: Into<PathBuf>>(&mut self, path: T) -> Result<(), WatchError> {
        let mut true_path = path.into();
        if !self.rules.matches(&true_path) {
            return Err(WatchError::Excluded(format!("{:?} has been excluded", &true_path)));
        }

        let true_path_str = match true_path.to_str() {
            Some(v) => v.to_string(),
            None => {
                return Err(WatchError::InvalidPath(format!("{:?} is an invalid path", &true_path)));
            },
        };

        if true_path.is_file() {
            self.watched_files.insert(true_path.clone());
            true_path.pop();
        } else {
            match glob(&format!("{:?}/**/*", true_path_str)) {
                Ok(v) => {
                    for entry in v {
                        match entry {
                            Ok(final_v) => {
                                self.watched_files.insert(final_v);
                            },
                            Err(_) => {
                                // Couldn't read the entry for some reason.. maybe we should throw
                                // an error?
                            },
                        }
                    }
                },
                Err(_) => {
                    return Err(WatchError::InvalidPath(format!("{:?} is an invalid path", &true_path)));
                },
            }
        }

        self.watched_dirs.push(true_path_str.to_string());
        Ok(())
    }

    pub fn init(&mut self) -> () {
        if self.fsevent_thread.is_some() {
            warn!("fsevent watcher has already been initialized");
            return;
        }

        let sender_clone = self.fsevent_sender.clone();
        let watched_dirs_clone = self.watched_dirs.clone();
        self.fsevent_thread = Some(thread::spawn(move || {
            fsevent::FsEvent::new(watched_dirs_clone).observe(sender_clone);
        }));
    }

    fn handle_create(&mut self, path: &PathBuf, events: &mut Vec<Event>) -> () {
        if !path.exists() || self.watched_files.contains(path) {
            return;
        }

        if !self.rules.matches(&path) {
            info!("excluded {:?} from watcher", &path);
            return;
        }

        self.watched_files.insert(path.clone());
        events.push(Event::Create(path.clone()));
    }

    fn handle_modify(&self, path: &PathBuf, events: &mut Vec<Event>) -> () {
        if !self.watched_files.contains(path) {
            return;
        }

        events.push(Event::Write(path.clone()));
    }

    fn handle_move(&mut self, path: &PathBuf, events: &mut Vec<Event>) -> () {
        if !path.exists() {
            self.handle_delete(&path, events);
        } else {
            self.handle_create(&path, events);
        }
    }

    fn handle_delete(&mut self, path: &PathBuf, events: &mut Vec<Event>) -> () {
        if !self.watched_files.contains(path) || path.exists() {
            return;
        }

        self.watched_files.remove(path);
        events.push(Event::Delete(path.clone()));
    }
}

pub struct MacOsEventStream {
    watcher: MacOsWatcher,
    events: Vec<Event>,
}

impl Stream for MacOsEventStream {
    type Item = Event;
    type Error = StreamError;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        while self.events.is_empty() {
            // Rebuild our empty vector to resize any allocated space
            mem::replace(&mut self.events, Vec::new());

            // Check if there is an event in the buffer
            let raw_event = match self.watcher.fsevent_receiver.try_recv() {
                Ok(v) => v,
                Err(_) => try_ready!(blocking(|| { // No event so block on it
                    self.watcher.fsevent_receiver.recv()
                }))?,
            };

            // Keep pulling events until we get a file event
            if !raw_event.flag.contains(fsevent::StreamFlags::IS_FILE) {
                continue;
            }

            let path = PathBuf::from(raw_event.path.clone());
            if raw_event.flag.contains(fsevent::StreamFlags::ITEM_REMOVED) {
                self.watcher.handle_delete(&path, &mut self.events);
            }
            if raw_event.flag.contains(fsevent::StreamFlags::ITEM_RENAMED) {
                self.watcher.handle_move(&path, &mut self.events);
            }
            if raw_event.flag.contains(fsevent::StreamFlags::ITEM_MODIFIED) {
                self.watcher.handle_modify(&path, &mut self.events);
            }
            if raw_event.flag.contains(fsevent::StreamFlags::ITEM_CREATED) {
                self.watcher.handle_create(&path, &mut self.events);
            }
        }

        // Pop events from the non empty event vector
        match self.events.pop() {
            Some(e) => Ok(Async::Ready(Some(e))),
            None => {
                // Not sure how we got here
                self.poll()
            }
        }
    }
}
