use crate::checksum_tree::ChecksumTree;
use std::{error::Error, io::Read, path::Path};

pub mod ftp;
pub mod local;

#[async_trait::async_trait]
pub trait Transport {
    async fn get_last_checksum(
        &mut self,
        checksum_filepath: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error>>;

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error>>;

    async fn upload(
        &mut self,
        filename: &Path,
        read: Box<dyn Read + Send>,
    ) -> Result<u64, Box<dyn Error>>;

    async fn remove(&mut self, pathname: &Path) -> Result<(), Box<dyn Error>>;

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error>>;
}
