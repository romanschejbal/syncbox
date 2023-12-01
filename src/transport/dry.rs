use std::{error::Error, io::Cursor, path::Path};

use tokio::io::AsyncRead;

use super::Transport;
use crate::checksum_tree::ChecksumTree;

pub struct DryTransport;

#[async_trait::async_trait]
impl Transport for DryTransport {
    async fn read_last_checksum(
        &mut self,
        _checksum_filename: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error + Send + Sync + 'static>> {
        Ok(ChecksumTree::default())
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
        _filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        Ok(Vec::new())
    }

    async fn mkdir(&mut self, _path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(())
    }

    async fn write(
        &mut self,
        _filename: &Path,
        _reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        Ok(file_size)
    }

    async fn remove(
        &mut self,
        _pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(())
    }
}
