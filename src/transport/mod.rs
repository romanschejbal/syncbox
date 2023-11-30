use crate::checksum_tree::ChecksumTree;
use std::{error::Error, io::Cursor, path::Path};
use tokio::io::AsyncRead;

pub mod ftp;
pub mod local;
pub mod s3;

#[async_trait::async_trait(?Send)]
pub trait Transport {
    async fn read_last_checksum(
        &mut self,
        checksum_filename: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error + Send + Sync + 'static>> {
        Ok(self
            .read(checksum_filename)
            .await
            .ok()
            .map(|bytes| ChecksumTree::from_gzip(&bytes))
            .transpose()?
            .unwrap_or_default())
    }

    async fn write_last_checksum(
        &mut self,
        checksum_filename: &Path,
        checksum_tree: &ChecksumTree,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let json = checksum_tree.to_gzip()?;
        let file_size = json.len();
        let cursor = Cursor::new(json);
        self.write(checksum_filename, Box::new(cursor), file_size as u64)
            .await
    }

    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>>;

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;

    async fn write(
        &mut self,
        filename: &Path,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>>;

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;
}
