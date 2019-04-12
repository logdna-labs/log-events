use glob::Pattern;
use regex::Regex;
use std::fmt::Debug;
use std::path::PathBuf;

pub trait Rule<T>: Debug {
    fn matches(&self, item: &T) -> bool;
}

#[derive(Debug)]
pub struct RegexRule {
    regex: Regex
}

impl Rule<PathBuf> for RegexRule {
    fn matches(&self, item: &PathBuf) -> bool {
        if let Some(s) = item.to_str() {
            self.regex.is_match(s)
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct GlobRule {
    pattern: Pattern
}

impl Rule<PathBuf> for GlobRule {
    fn matches(&self, item: &PathBuf) -> bool {
        if let Some(s) = item.to_str() {
            self.pattern.matches(s)
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct Rules {
    include: Vec<Box<Rule<PathBuf> + Send>>,
    exclude: Vec<Box<Rule<PathBuf> + Send>>,
}

impl Rules {
    pub fn new() -> Self {
        Rules {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
    pub fn matches(&self, item: &PathBuf) -> bool {
        for include_rule in &self.include {
            if !include_rule.matches(item) {
                return false;
            }
        }
        for exclude_rule in &self.exclude {
            if exclude_rule.matches(item) {
                return false;
            }
        }
        true
    }
}

