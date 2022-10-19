use crate::checksum_tree::ChecksumTree;
use std::{error::Error, io::Read, path::Path};

pub mod ftp;
pub mod local;

#[async_trait::async_trait(?Send)]
pub trait Transport {
    async fn get_last_checksum(
        &mut self,
        checksum_filename: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error>> {
        Ok(self
            .read(checksum_filename)
            .await
            .ok()
            .map(|bytes| serde_json::from_slice::<ChecksumTree>(&bytes))
            .transpose()?
            .unwrap_or_default())
    }

    async fn read(&mut self, filename: &Path) -> Result<Vec<u8>, Box<dyn Error>>;

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error>>;

    async fn write(
        &mut self,
        filename: &Path,
        read: Box<dyn Read + Send>,
    ) -> Result<u64, Box<dyn Error>>;

    async fn remove(&mut self, pathname: &Path) -> Result<(), Box<dyn Error>>;

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error>>;
}
