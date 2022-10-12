use crate::checksum_tree::{ChecksumElement, ChecksumTree};
use std::{collections::VecDeque, path::PathBuf};

#[derive(Debug)]
pub enum Action {
    Mkdir(PathBuf),
    Put(PathBuf),
    Remove(String),
}

pub struct Reconciler {}

impl Reconciler {
    pub fn reconcile(prev: ChecksumTree, next: &ChecksumTree) -> Vec<Action> {
        let mut previous_checksum = prev.get_root();
        let mut actions = vec![];
        let mut to_reconcile = VecDeque::from([(vec![], &**next)]);
        while !to_reconcile.is_empty() {
            let (next_depth, next) = to_reconcile.pop_front().unwrap();
            match next {
                ChecksumElement::Root(dir) | ChecksumElement::Directory(dir) => {
                    // create vec of path to do lookup for
                    for (path, element) in dir {
                        let mut new_depth = next_depth.clone();
                        new_depth.push(path);
                        to_reconcile.push_back((new_depth, element));
                    }
                }
                ChecksumElement::File(new_checksum) => {
                    // see if we had it in previous - create directories
                    let mut stack = vec![previous_checksum];
                    let mut path = vec![];
                    for key in next_depth.iter().take(next_depth.len() - 1) {
                        path.push(*key);
                        let currently_searching = stack.last_mut().unwrap();
                        match currently_searching {
                            ChecksumElement::Root(dir) | ChecksumElement::Directory(dir) => {
                                if let Some(next_to_search) = dir.remove(*key) {
                                    stack.push(next_to_search);
                                } else {
                                    let new_dir = ChecksumElement::Directory(Default::default());
                                    stack.push(new_dir);
                                    // ignore "." directories
                                    if path.len() > 1 {
                                        actions.push(Action::Mkdir(path.iter().collect()));
                                    }
                                }
                            }
                            _ => (),
                        };
                    }
                    // check for file or create file
                    let leaf = stack.last_mut().unwrap();
                    match leaf {
                        ChecksumElement::Root(dir) | ChecksumElement::Directory(dir) => {
                            let filename = *next_depth.last().unwrap();

                            if let Some(element) = dir.remove(filename) {
                                let matches = match element {
                                    ChecksumElement::File(previous_checksum) => {
                                        previous_checksum == *new_checksum
                                    }
                                    _ => unreachable!(),
                                };
                                if !matches {
                                    actions.push(Action::Put(next_depth.iter().collect()));
                                }
                            } else {
                                dir.insert(
                                    filename.clone(),
                                    ChecksumElement::File(new_checksum.clone()),
                                );
                                actions.push(Action::Put(next_depth.iter().collect()));
                            }
                        }
                        _ => unreachable!(),
                    };

                    // build back into tree
                    while stack.len() > 1 {
                        let child = stack.pop().unwrap();
                        let parent = stack.last_mut().unwrap();
                        match parent {
                            ChecksumElement::Root(dir) | ChecksumElement::Directory(dir) => {
                                dir.insert(path.pop().unwrap().clone(), child);
                            }
                            _ => (),
                        }
                    }
                    previous_checksum = stack.pop().unwrap();
                }
            }
        }
        actions
    }
}
