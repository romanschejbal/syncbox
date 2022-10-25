use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    path::Path,
};

#[derive(Serialize, Deserialize, Debug)]
pub enum ChecksumElement {
    #[serde(rename = "d", alias = "Directory")]
    Directory(HashMap<String, ChecksumElement>),
    #[serde(rename = "f", alias = "File")]
    File(String),
}

impl Default for ChecksumElement {
    fn default() -> Self {
        Self::Directory(HashMap::default())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChecksumTree {
    #[serde(default)]
    version: String,
    root: Option<ChecksumElement>,
}

impl ChecksumTree {
    fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            root: Some(ChecksumElement::default()),
        }
    }

    pub fn get_root(&mut self) -> &mut Option<ChecksumElement> {
        &mut self.root
    }

    pub fn get_version(&self) -> &str {
        &self.version
    }

    /// Used for when there was error uploading files
    pub fn remove_at(&mut self, path: &Path) {
        let root_dir = self.get_root().take().unwrap();
        let mut stack = vec![root_dir];
        let mut path_str = vec![];
        for component in path.iter() {
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
        self.root = Some(ChecksumElement::Directory(hashmap));
    }
}

impl Default for ChecksumTree {
    fn default() -> Self {
        ChecksumTree::new()
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
                    ChecksumElement::Directory(mut parent_map) => {
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
            root: Some(stack.pop().unwrap()),
            ..Default::default()
        }
    }
}

impl Deref for ChecksumTree {
    type Target = Option<ChecksumElement>;
    fn deref(&self) -> &Self::Target {
        &self.root
    }
}

impl DerefMut for ChecksumTree {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.root
    }
}
