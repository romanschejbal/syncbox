use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    error::Error,
    ops::{Deref, DerefMut},
    path::Path,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ChecksumElement {
    #[serde(alias = "d")]
    Directory(HashMap<String, ChecksumElement>),
    #[serde(alias = "f")]
    File(String),
}

impl Default for ChecksumElement {
    fn default() -> Self {
        Self::Directory(HashMap::default())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

    /// Used for when there was an error while uploading files
    pub fn remove_at(&mut self, path: &Path) {
        if let Some(ChecksumElement::Directory(root_dir)) = self.root.as_mut() {
            let mut current_dir = root_dir;
            let components: Vec<_> = path
                .iter()
                .map(|c| c.to_string_lossy().to_string())
                .collect();

            for (i, component) in components.iter().enumerate() {
                // Check if we are at the last component (file or empty directory)
                if i == components.len() - 1 {
                    current_dir.remove(component);
                    return;
                }

                // Navigate to the next directory
                if let Some(ChecksumElement::Directory(next_dir)) = current_dir.get_mut(component) {
                    current_dir = next_dir;
                } else {
                    // Path does not exist, nothing to remove
                    return;
                }
            }
        }
    }

    pub fn to_gzip(&self) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        serde_json::to_writer(&mut encoder, self).unwrap();
        Ok(encoder.finish()?)
    }

    pub fn from_gzip(bytes: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let mut decoder = flate2::read::GzDecoder::new(bytes);
        Ok(serde_json::from_reader(&mut decoder)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_at() {
        let mut checksum: ChecksumTree = serde_json::from_str(
            r#"{
           "version": "0.3.0",
           "root": {
             "Directory": {
               ".": {
                 "Directory": {
                   "DSC05947.ARW": {
                     "File": "a4849b4f83f996ef9ce68b9f8561db4a991ab5f9dce3c52a45267c8e274bb73a"
                   },
                   "DSC06087.ARW": {
                     "File": "aed5627230975590635e2f4809b6aa1f8ccc7f536fa97e08c76824ba093fbca3"
                   },
                   "DSC05953.ARW": {
                     "File": "3e90e6673d78a1b12f51f417c4bfb555b5040557bef249a44faadc336273a55e"
                   }
                 }
               }
             }
           }
        }"#,
        )
        .unwrap();
        checksum.remove_at(Path::new("./DSC06087.ARW"));
        checksum.remove_at(Path::new("./DSC05953.ARW"));
        assert_eq!(
            serde_json::to_string(&checksum).unwrap(),
            r#"{"version":"0.3.0","root":{"Directory":{".":{"Directory":{"DSC05947.ARW":{"File":"a4849b4f83f996ef9ce68b9f8561db4a991ab5f9dce3c52a45267c8e274bb73a"}}}}}}"#
        );
    }

    #[test]
    fn remove_at_similar() {
        let mut checksum: ChecksumTree = serde_json::from_str(
            r#"{
           "version": "0.3.0",
           "root": {
             "Directory": {
               "dirrr": {
                 "Directory": {
                   "DSC05947.ARW": {
                     "File": "a4849b4f83f996ef9ce68b9f8561db4a991ab5f9dce3c52a45267c8e274bb73a"
                   },
                   "DSC06087.ARW": {
                     "File": "aed5627230975590635e2f4809b6aa1f8ccc7f536fa97e08c76824ba093fbca3"
                   },
                   "DSC05953.ARW": {
                     "File": "3e90e6673d78a1b12f51f417c4bfb555b5040557bef249a44faadc336273a55e"
                   }
                 }
               }
             }
           }
        }"#,
        )
        .unwrap();
        checksum.remove_at(Path::new("dirrr/DSC06087.ARW"));
        checksum.remove_at(Path::new("dirrr/DSC05953.ARW"));
        assert_eq!(
            serde_json::to_string(&checksum).unwrap(),
            r#"{"version":"0.3.0","root":{"Directory":{"dirrr":{"Directory":{"DSC05947.ARW":{"File":"a4849b4f83f996ef9ce68b9f8561db4a991ab5f9dce3c52a45267c8e274bb73a"}}}}}}"#
        );
    }

    #[test]
    fn from_hashmap_single_file() {
        let mut map = HashMap::new();
        map.insert("./file.txt".to_string(), "abc123".to_string());
        let tree: ChecksumTree = map.into();

        match tree.as_ref().unwrap() {
            ChecksumElement::Directory(root) => {
                let dot = root.get(".").unwrap();
                match dot {
                    ChecksumElement::Directory(dir) => match dir.get("file.txt").unwrap() {
                        ChecksumElement::File(hash) => assert_eq!(hash, "abc123"),
                        _ => panic!("expected File"),
                    },
                    _ => panic!("expected Directory"),
                }
            }
            _ => panic!("expected Directory"),
        }
    }

    #[test]
    fn from_hashmap_multiple_files_same_directory() {
        let mut map = HashMap::new();
        map.insert("./a.txt".to_string(), "hash_a".to_string());
        map.insert("./b.txt".to_string(), "hash_b".to_string());
        let tree: ChecksumTree = map.into();

        match tree.as_ref().unwrap() {
            ChecksumElement::Directory(root) => {
                let dot = match root.get(".").unwrap() {
                    ChecksumElement::Directory(d) => d,
                    _ => panic!("expected Directory"),
                };
                assert_eq!(dot.len(), 2);
                assert!(
                    matches!(dot.get("a.txt"), Some(ChecksumElement::File(h)) if h == "hash_a")
                );
                assert!(
                    matches!(dot.get("b.txt"), Some(ChecksumElement::File(h)) if h == "hash_b")
                );
            }
            _ => panic!("expected Directory"),
        }
    }

    #[test]
    fn from_hashmap_deeply_nested() {
        let mut map = HashMap::new();
        map.insert("./a/b/c/file.txt".to_string(), "deep_hash".to_string());
        let tree: ChecksumTree = map.into();

        // Navigate: root -> "." -> "a" -> "b" -> "c" -> "file.txt"
        let root = match tree.as_ref().unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        let dot = match root.get(".").unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        let a = match dot.get("a").unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        let b = match a.get("b").unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        let c = match b.get("c").unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        assert!(matches!(c.get("file.txt"), Some(ChecksumElement::File(h)) if h == "deep_hash"));
    }

    #[test]
    fn from_hashmap_files_in_different_directories() {
        let mut map = HashMap::new();
        map.insert("./dir1/file1.txt".to_string(), "hash1".to_string());
        map.insert("./dir2/file2.txt".to_string(), "hash2".to_string());
        let tree: ChecksumTree = map.into();

        let root = match tree.as_ref().unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        let dot = match root.get(".").unwrap() {
            ChecksumElement::Directory(d) => d,
            _ => panic!(),
        };
        assert_eq!(dot.len(), 2);
        assert!(dot.contains_key("dir1"));
        assert!(dot.contains_key("dir2"));
    }

    #[test]
    fn from_hashmap_roundtrip_through_reconciler() {
        use crate::reconciler::Reconciler;

        let mut map = HashMap::new();
        map.insert("./dir/file.txt".to_string(), "hash1".to_string());
        map.insert("./other/nested/file.txt".to_string(), "hash2".to_string());
        let tree1: ChecksumTree = map.clone().into();
        let tree2: ChecksumTree = map.into();

        let actions = Reconciler::reconcile(tree1, &tree2).unwrap();
        assert!(
            actions.is_empty(),
            "identical trees should produce zero actions"
        );
    }

    #[test]
    fn gzip_roundtrip() {
        let mut map = HashMap::new();
        map.insert("./dir/file.txt".to_string(), "hash123".to_string());
        let tree: ChecksumTree = map.into();

        let compressed = tree.to_gzip().unwrap();
        let restored = ChecksumTree::from_gzip(&compressed).unwrap();

        assert_eq!(
            serde_json::to_string(&tree).unwrap(),
            serde_json::to_string(&restored).unwrap()
        );
    }

    #[test]
    fn gzip_roundtrip_empty_tree() {
        let tree = ChecksumTree::default();

        let compressed = tree.to_gzip().unwrap();
        let restored = ChecksumTree::from_gzip(&compressed).unwrap();

        assert_eq!(
            serde_json::to_string(&tree).unwrap(),
            serde_json::to_string(&restored).unwrap()
        );
    }
}
