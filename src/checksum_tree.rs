use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Deref, path::Path};

#[derive(Serialize, Deserialize, Debug)]
pub enum ChecksumElement {
    Root(HashMap<String, ChecksumElement>),
    #[serde(rename = "d", alias = "Directory")]
    Directory(HashMap<String, ChecksumElement>),
    #[serde(rename = "f", alias = "File")]
    File(String),
}

impl Default for ChecksumElement {
    fn default() -> Self {
        Self::Root(HashMap::default())
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ChecksumTree {
    root: ChecksumElement,
}

impl ChecksumTree {
    pub fn get_root(self) -> ChecksumElement {
        self.root
    }

    /// Used for when there was error uploading files
    pub fn remove_at(&mut self, path: &Path) {
        let root_dir = match self.root {
            ChecksumElement::Root(ref mut dir) | ChecksumElement::Directory(ref mut dir) => {
                dir.remove(".").unwrap()
            }
            _ => unreachable!(),
        };
        let mut stack = vec![root_dir];
        let mut path_str = vec![];
        for component in path.iter().skip(1) {
            path_str.push(component);
            match stack.last_mut().unwrap() {
                ChecksumElement::Directory(dir) => {
                    if let Some(next) = dir.remove(&component.to_string_lossy().to_string()) {
                        stack.push(next);
                    }
                }
                _ => unreachable!(),
            }
        }
        stack.pop();
        path_str.pop();

        while stack.len() > 1 {
            let child = stack.pop().unwrap();
            let parent = stack.last_mut().unwrap();
            match parent {
                ChecksumElement::Directory(ref mut dir) => {
                    dir.insert(path_str.pop().unwrap().to_string_lossy().to_string(), child);
                }
                _ => unreachable!(),
            }
        }

        assert!(stack.len() == 1);
        let mut hashmap: HashMap<String, ChecksumElement> = Default::default();
        hashmap.insert(".".to_string(), stack.pop().unwrap());
        self.root = ChecksumElement::Root(hashmap);
    }
}

impl From<HashMap<String, String>> for ChecksumTree {
    fn from(map: HashMap<String, String>) -> Self {
        let root_map = Default::default();
        let mut stack: Vec<ChecksumElement> = vec![root_map];
        for (path_str, checksum) in map {
            let path = Path::new(&path_str);
            // create nested maps and put on the stack
            for component in path {
                let parent = stack.pop().unwrap();
                match parent {
                    ChecksumElement::Root(mut parent_map)
                    | ChecksumElement::Directory(mut parent_map) => {
                        let map = parent_map
                            .remove(component.to_string_lossy().as_ref())
                            .unwrap_or_default();
                        stack.push(ChecksumElement::Directory(parent_map));
                        stack.push(map);
                    }
                    _ => unreachable!(),
                }
            }
            // ditch last part which is filename
            stack.pop();
            // fill the last map with file
            let map = stack.pop().unwrap();
            let map = match map {
                ChecksumElement::Directory(mut dir) => {
                    dir.insert(
                        path.file_name().unwrap().to_string_lossy().to_string(),
                        ChecksumElement::File(checksum),
                    );
                    dir
                }
                _ => unreachable!(),
            };
            stack.push(ChecksumElement::Directory(map));
            // pop back into main root map
            for component in path.iter().rev().skip(1) {
                let component_str: String = component.to_string_lossy().to_string();
                let dir = stack.pop().unwrap();
                let parent = stack.pop().unwrap();
                let parent = match parent {
                    ChecksumElement::Directory(mut parent) => {
                        parent.insert(component_str, dir);
                        parent
                    }
                    _ => unreachable!(),
                };
                stack.push(ChecksumElement::Directory(parent));
            }
        }

        assert!(stack.len() == 1);

        Self {
            root: stack.pop().unwrap(),
        }
    }
}

impl Deref for ChecksumTree {
    type Target = ChecksumElement;
    fn deref(&self) -> &Self::Target {
        &self.root
    }
}
