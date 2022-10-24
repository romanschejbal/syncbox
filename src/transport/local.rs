use super::Transport;
use std::{
    error::Error,
    path::{Path, PathBuf},
    process::Command,
};
use tokio::{fs, io::AsyncRead};

pub struct LocalFilesystem {
    dir: PathBuf,
}

impl Default for LocalFilesystem {
    fn default() -> Self {
        let dir =
            String::from_utf8(Command::new("mktemp").arg("-d").output().unwrap().stdout).unwrap();
        println!("Created temp dir: {dir}");
        Self {
            dir: dir.trim().into(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Transport for LocalFilesystem {
    async fn read(&mut self, filename: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut path = self.dir.clone();
        path.push(filename);
        Ok(fs::read(path).await?)
    }

    async fn mkdir(&mut self, dir_path: &Path) -> Result<(), Box<dyn Error>> {
        let mut path = self.dir.clone();
        path.push(dir_path);
        tokio::fs::create_dir(path).await?;
        Ok(())
    }

    async fn write(
        &mut self,
        filename: &Path,
        source: Box<dyn AsyncRead>,
        _progress_update_callback: Box<dyn Fn(u64)>,
    ) -> Result<u64, Box<dyn Error>> {
        let mut dir = self.dir.clone();
        dir.push(filename);
        let mut file = tokio::fs::File::create(dir).await?;
        let mut source = Box::into_pin(source);
        Ok(tokio::io::copy(&mut source, &mut file).await?)
    }

    async fn remove(&mut self, _pathname: &Path) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
