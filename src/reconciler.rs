use crate::checksum_tree::{ChecksumElement, ChecksumTree};
use std::error::Error;
use std::{collections::VecDeque, ops::Deref, path::PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Mkdir(PathBuf),
    Put(PathBuf),
    Remove(PathBuf),
}

pub struct Reconciler {}

impl Reconciler {
    pub fn reconcile(
        mut prev: ChecksumTree,
        next: &ChecksumTree,
    ) -> Result<Vec<Action>, Box<dyn Error + Send + Sync + 'static>> {
        check_version(prev.get_version(), next.get_version())?;
        let mut previous_checksum = prev.get_root().take().unwrap_or_default();
        let mut actions = vec![];
        let root = next.deref().as_ref().unwrap();
        let mut to_reconcile = VecDeque::from([(vec![], root)]);
        while !to_reconcile.is_empty() {
            let (next_depth, next) = to_reconcile.pop_front().unwrap();
            match next {
                ChecksumElement::Directory(dir) => {
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
                        if let ChecksumElement::Directory(dir) = currently_searching {
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
                        };
                    }

                    // check for file or create file
                    let leaf = stack.last_mut().unwrap();
                    match leaf {
                        ChecksumElement::Directory(dir) => {
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
                                actions.push(Action::Put(next_depth.iter().collect()));
                            }
                        }
                        _ => unreachable!(),
                    };

                    // build back into tree
                    while stack.len() > 1 {
                        let child = stack.pop().unwrap();
                        let parent = stack.last_mut().unwrap();
                        if let ChecksumElement::Directory(dir) = parent {
                            dir.insert(path.pop().unwrap().clone(), child);
                        }
                    }
                    previous_checksum = stack.pop().unwrap();
                }
            }
        }

        // collect files that left in previous and mark them to be removed
        let mut stack: Vec<(PathBuf, &ChecksumElement)> = vec![("".into(), &previous_checksum)];
        while !stack.is_empty() {
            let (path, current) = stack.pop().unwrap();
            match current {
                ChecksumElement::Directory(dir) => {
                    dir.iter().for_each(|(dir_name, element)| {
                        let mut new_path = path.clone();
                        new_path.push(dir_name);
                        stack.push((new_path, element));
                    });
                }
                ChecksumElement::File(_) => actions.push(Action::Remove(path)),
            }
        }

        Ok(actions)
    }
}

/// Panics if previous version is newer
fn check_version(prev: &str, next: &str) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    if next < prev {
        return Err(format!(
            "Your version of syncbox seems outdated, please update to at least {prev}"
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::collections::HashMap;

    #[test]
    fn empty() {
        let prev = ChecksumTree::default();
        let next = ChecksumTree::default();
        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.is_empty());
    }

    #[test]
    fn insert_into_root() {
        let prev = ChecksumTree::default();
        let mut next = HashMap::new();
        next.insert("./file.txt".to_string(), "sha256hash".to_string());
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Put("./file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn insert_one_level_deep_with_create_directory() {
        let prev = ChecksumTree::default();
        let mut next = HashMap::new();
        next.insert("./direktory/file.txt".to_string(), "sha256hash".to_string());
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 2);
        diff.into_iter()
            .zip(vec![
                Action::Mkdir("./direktory".into()),
                Action::Put("./direktory/file.txt".into()),
            ])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn insert_two_levels_deep_with_create_directories() {
        let prev = ChecksumTree::default();
        let mut next = HashMap::new();
        next.insert(
            "./direktory/nested/file.txt".to_string(),
            "sha256hash".to_string(),
        );
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 3);
        diff.into_iter()
            .zip(vec![
                Action::Mkdir("./direktory".into()),
                Action::Mkdir("./direktory/nested".into()),
                Action::Put("./direktory/nested/file.txt".into()),
            ])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn update_in_root() {
        let mut prev = HashMap::new();
        prev.insert("./file.txt".to_string(), "sha256hash".to_string());
        let prev: ChecksumTree = prev.into();
        let mut next = HashMap::new();
        next.insert("./file.txt".to_string(), "sha256hashThatsNew".to_string());
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Put("./file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn update_one_level_deep_with_create_directory() {
        let mut prev = HashMap::new();
        prev.insert("./direktory/file.txt".to_string(), "sha256hash".to_string());
        let prev: ChecksumTree = prev.into();
        let mut next = HashMap::new();
        next.insert(
            "./direktory/file.txt".to_string(),
            "sha256hashThatsNew".to_string(),
        );
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Put("./direktory/file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn update_two_levels_deep_with_create_directories() {
        let mut prev = HashMap::new();
        prev.insert(
            "./direktory/nested/file.txt".to_string(),
            "sha256hash".to_string(),
        );
        let prev: ChecksumTree = prev.into();
        let mut next = HashMap::new();
        next.insert(
            "./direktory/nested/file.txt".to_string(),
            "sha256hashThatsNew".to_string(),
        );
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Put("./direktory/nested/file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn remove_from_root() {
        let mut prev = HashMap::new();
        prev.insert("./file.txt".to_string(), "sha256hash".to_string());
        let prev: ChecksumTree = prev.into();
        let next: ChecksumTree = ChecksumTree::default();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Remove("./file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn remove_from_one_level_deep() {
        let mut prev = HashMap::new();
        prev.insert("./direktory/file.txt".to_string(), "sha256hash".to_string());
        let prev: ChecksumTree = prev.into();
        let next: ChecksumTree = ChecksumTree::default();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 1);
        diff.into_iter()
            .zip(vec![Action::Remove("./direktory/file.txt".into())])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn all_together() {
        let mut prev = HashMap::new();
        prev.insert(
            "./direktory/file1.txt".to_string(),
            "wontChange".to_string(),
        );
        prev.insert(
            "./direktory2/nested/file2.txt".to_string(),
            "sha256hash".to_string(),
        );
        prev.insert(
            "./direktory2/other/file3.txt".to_string(),
            "will-be-removed".to_string(),
        );
        let prev: ChecksumTree = prev.into();
        let mut next = HashMap::new();
        next.insert(
            "./direktory/file1.txt".to_string(),
            "wontChange".to_string(),
        );
        next.insert(
            "./direktory2/nested/file2.txt".to_string(),
            "sha256hashThatsNew".to_string(),
        );
        let next: ChecksumTree = next.into();

        let diff = Reconciler::reconcile(prev, &next).unwrap();

        assert!(diff.len() == 2);
        diff.into_iter()
            .zip(vec![
                Action::Put("./direktory2/nested/file2.txt".into()),
                Action::Remove("./direktory2/other/file3.txt".into()),
            ])
            .for_each(|(a, b)| assert_eq!(a, b));
    }

    #[test]
    fn version_equal_ok() {
        assert_eq!(check_version("0.1.0", "0.1.1").ok(), Some(()));
    }

    #[test]
    fn version_older_ok() {
        assert_eq!(check_version("0.1.2", "0.1.3").ok(), Some(()));
        assert_eq!(check_version("0.1.2", "1.1.3").ok(), Some(()));
        assert_eq!(check_version("10.1.2", "10.10.3").ok(), Some(()));
    }

    #[test]
    fn version_newer_not_ok() {
        assert_eq!(
            check_version("0.1.2", "0.1.1").map_err(|e| e.to_string()),
            Err(
                "Your version of syncbox seems outdated, please update to at least 0.1.2"
                    .to_string()
            )
        );
    }
}
