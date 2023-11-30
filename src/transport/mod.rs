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
            .map(|bytes| serde_json::from_slice::<ChecksumTree>(&bytes))
            .transpose()?
            .unwrap_or_default())
    }

    async fn write_last_checksum(
        &mut self,
        checksum_filename: &Path,
        checksum_tree: &ChecksumTree,
        progress_update_callback: Box<dyn Fn(u64)>,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let json = serde_json::to_string_pretty(checksum_tree)?;
        let file_size = json.len();
        let cursor = Cursor::new(json);
        self.write(
            checksum_filename,
            Box::new(cursor),
            progress_update_callback,
            file_size as u64,
        )
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
        read: Box<dyn AsyncRead + Unpin + Send>,
        progress_update_callback: Box<dyn Fn(u64)>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>>;

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;
}
