#[macro_use]
extern crate lazy_static;


use std::ffi::OsStr;
use std::fs::read_link;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::{Event};

use glob::glob;
use hashbrown::HashMap;
use inotify::{
    Event as RawEvent, EventMask, Inotify, WatchDescriptor, WatchMask,
};

lazy_static! {
    static ref DIR_WATCH_MASK: WatchMask = WatchMask::CREATE |  WatchMask::DELETE;
    static ref FILE_WATCH_MASK: WatchMask =   WatchMask::MOVE_SELF | WatchMask::MODIFY;
}

pub struct LinuxEventStreamer {
    watch_descriptor_map: HashMap<WatchDescriptor, PathBuf>,
    path_map: HashMap<PathBuf, WatchDescriptor>,
    inotify: Inotify,
    options: Options,
}

impl LinuxEventStreamer {
    pub fn new(options: Options) -> LinuxEventStreamer {
        LinuxEventStreamer {
            watch_descriptor_map: HashMap::new(),
            path_map: HashMap::new(),
            inotify: Inotify::init().unwrap(),
            options,
        }
    }

    fn handle_create(&mut self, path: &PathBuf, raw_event: RawEvent<&OsStr>, events: &mut Vec<Event>) {
        if let Some(path) = raw_event.name.map(|s| path.join(s)) {
            if !raw_event.mask.contains(EventMask::ISDIR) {
                if let Some(s) = path.to_str() { self.add(s) }
                if check_path(&self.options, &path) {
                    events.push(Event::Create(path));
                }
            } else {
                if let Some(s) = path.to_str() { self.add(s) }
                if let Some(s) = path.join("**/*").to_str() {
                    self.add(s);
                    match glob(s) {
                        Ok(paths) => paths.filter_map(|r| r.ok())
                            .filter(|p| p.is_file())
                            .filter(|p| check_path(&self.options, p))
                            .for_each(|f| events.push(Event::Create(f))),
                        Err(e) => { error!("glob error {:?}: {:?}", &path, &e) }
                    }
                }
            }
        }
    }

    fn handle_delete(&mut self, path: &PathBuf, raw_event: RawEvent<&OsStr>, events: &mut Vec<Event>) {
        let path = match raw_event.name.map(|s| path.join(s)) {
            Some(v) => v,
            _ => { return; }
        };

        if let Some(wd) = self.path_map.get(&path) {
            info!("removed {:?} -> {:?}from watcher", &path, &wd);
            self.watch_descriptor_map.remove(wd);
            self.path_map.remove(&path);
            info!("now watching ({},{}) items", self.path_map.len(), self.watch_descriptor_map.len());
        }

        if !raw_event.mask.contains(EventMask::ISDIR) {
            events.push(Event::Delete(path))
        }
    }

    fn handle_modify(&mut self, path: PathBuf, events: &mut Vec<Event>) {
        if path.is_file() {
            events.push(Event::Write(path))
        }
    }

    fn handle_move(&mut self, path: PathBuf, events: &mut Vec<Event>) {
        info!("{:?} was moved!", &path);
        if let Some(wd) = self.path_map.get(&path) {
            info!("removed {:?} -> {:?}from watcher", &path, &wd);
            if let Err(e) = self.inotify.rm_watch(wd.clone()) {
                error!("error removing moved file {:?}: {:?}", &path, &e)
            }
            self.watch_descriptor_map.remove(wd);
            self.path_map.remove(&path);
            info!("now watching ({},{}) items", self.path_map.len(), self.watch_descriptor_map.len());
            events.push(Event::Delete(path.clone()))
        }

        let real_path = read_link(&path).unwrap_or(path.clone());

        let start = Instant::now();
        let timeout = Duration::from_secs(5);
        let interval = Duration::from_millis(100);
        while !real_path.exists() {
            if start.elapsed() > timeout {
                error!("rotation failed for {:?}", &path);
                return;
            }
            sleep(interval)
        }

        if let Some(s) = path.to_str() { self.add(s) }
        if check_path(&self.options, &path) {
            events.push(Event::Create(path));
        }
    }

    pub fn add(&mut self, pattern: &str) {
        let paths = match glob(pattern) {
            Ok(v) => v,
            Err(e) => {
                error!("glob error: {:?}", &e);
                return;
            }
        };

        for path in paths {
            let path = match path {
                Ok(v) => v,
                Err(e) => {
                    error!("glob error: {:?}", &e);
                    continue;
                }
            };

            if !check_path(&self.options, &path) {
                info!("excluded {:?} from watcher", &path);
                continue;
            }

            let mask = if path.is_dir() {
                *DIR_WATCH_MASK
            } else {
                *FILE_WATCH_MASK
            };

            if self.path_map.contains_key(&path) {
                warn!("already watching {:?}", &path);
                continue;
            }

            let watch_descriptor = match self.inotify.add_watch(&path, mask) {
                Ok(v) => v,
                Err(e) => {
                    error!("inotify add error {:?}: {:?}", &path, &e);
                    continue;
                }
            };

            if self.watch_descriptor_map.contains_key(&watch_descriptor) {
                warn!("duplicate watch descriptor {:?}", &path);
                continue;
            }

            info!("added {:?} -> {:?} to watcher", &path, &watch_descriptor);
            self.path_map.insert(path.clone(), watch_descriptor.clone());
            self.watch_descriptor_map.insert(watch_descriptor, path);
            info!("now watching ({},{}) items", self.path_map.len(), self.watch_descriptor_map.len());
        }
    }
}

impl EventStreamer for LinuxEventStreamer {
    fn init(&mut self, patterns: &Vec<&str>) -> () {
        for pattern in patterns {
            self.add(pattern);
        }
    }

    fn stream(&mut self) -> Vec<Event> {
        let mut events = Vec::new();

        let mut buff = [0u8; 8_192];
        let raw_events = match self.inotify.read_events_blocking(&mut buff) {
            Ok(v) => v,
            Err(e) => {
                error!("error reading inotify events: {:?}", &e);
                return events;
            }
        };

        for raw_event in raw_events {
            if let Some(path) = self.watch_descriptor_map.get(&raw_event.wd).map(Clone::clone) {
                if raw_event.mask.contains(EventMask::CREATE) {
                    self.handle_create(&path, raw_event, &mut events);
                } else if raw_event.mask.contains(EventMask::DELETE) {
                    self.handle_delete(&path, raw_event, &mut events);
                } else if raw_event.mask.contains(EventMask::MODIFY) {
                    self.handle_modify(path, &mut events);
                } else if raw_event.mask.contains(EventMask::MOVE_SELF) {
                    self.handle_move(path, &mut events);
                }
            } else {
                warn!("event triggered for unknown watch descriptor {:?}", &raw_event)
            }
        }

        events
    }

    fn watched(&self) -> Vec<PathBuf> {
        let mut watched = Vec::with_capacity(self.watch_descriptor_map.len());
        for value in self.watch_descriptor_map.values() {
            watched.push(value.clone());
        }
        watched
    }
}
