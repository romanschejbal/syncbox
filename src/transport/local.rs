use super::Transport;
use std::{
    error::Error,
    path::{Path, PathBuf},
};
use tokio::{fs, io::AsyncRead};

pub struct LocalFilesystem {
    dir: PathBuf,
}

impl LocalFilesystem {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }
}

#[async_trait::async_trait]
impl Transport for LocalFilesystem {
    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let mut path = self.dir.clone();
        path.push(filename);
        Ok(fs::read(path).await?)
    }

    async fn mkdir(
        &mut self,
        dir_path: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut path = self.dir.clone();
        path.push(dir_path);
        tokio::fs::create_dir(path).await?;
        Ok(())
    }

    async fn write(
        &mut self,
        filename: &Path,
        source: Box<dyn AsyncRead + Unpin + Send>,
        _file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let mut dir = self.dir.clone();
        dir.push(filename);
        let mut file = tokio::fs::File::create(dir).await?;
        let mut source = Box::into_pin(source);
        Ok(tokio::io::copy(&mut source, &mut file).await?)
    }

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut path = self.dir.clone();
        path.push(pathname);
        Ok(tokio::fs::remove_file(path).await?)
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::io::AsyncRead;

    #[tokio::test]
    async fn write_and_read_file() {
        let tmp = TempDir::new().unwrap();
        let mut transport = LocalFilesystem::new(tmp.path());

        let data = b"hello world";
        let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(std::io::Cursor::new(data.to_vec()));
        transport
            .write(Path::new("test.txt"), reader, data.len() as u64)
            .await
            .unwrap();

        let result = transport.read(Path::new("test.txt")).await.unwrap();
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn mkdir_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let mut transport = LocalFilesystem::new(tmp.path());

        transport.mkdir(Path::new("subdir")).await.unwrap();
        assert!(tmp.path().join("subdir").is_dir());
    }

    #[tokio::test]
    async fn remove_deletes_file() {
        let tmp = TempDir::new().unwrap();
        let mut transport = LocalFilesystem::new(tmp.path());

        // Write a file first
        let data = b"to be removed";
        let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(std::io::Cursor::new(data.to_vec()));
        transport
            .write(Path::new("removeme.txt"), reader, data.len() as u64)
            .await
            .unwrap();

        transport.remove(Path::new("removeme.txt")).await.unwrap();
        assert!(!tmp.path().join("removeme.txt").exists());
    }

    #[tokio::test]
    async fn read_nonexistent_file_returns_error() {
        let tmp = TempDir::new().unwrap();
        let mut transport = LocalFilesystem::new(tmp.path());

        let result = transport.read(Path::new("does_not_exist.txt")).await;
        assert!(result.is_err());
    }
}
