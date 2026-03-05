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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_operations_succeed() {
        let mut transport = DryTransport;

        let read_result = transport.read(Path::new("file.txt")).await;
        assert!(read_result.is_ok());

        let mkdir_result = transport.mkdir(Path::new("dir")).await;
        assert!(mkdir_result.is_ok());

        let remove_result = transport.remove(Path::new("file.txt")).await;
        assert!(remove_result.is_ok());

        let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(Cursor::new(vec![1, 2, 3]));
        let write_result = transport.write(Path::new("file.txt"), reader, 3).await;
        assert!(write_result.is_ok());
        assert_eq!(write_result.unwrap(), 3);

        let close_result = Box::new(DryTransport).close().await;
        assert!(close_result.is_ok());
    }
}
