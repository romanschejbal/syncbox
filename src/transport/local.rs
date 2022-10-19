use super::Transport;
use rand::Rng;
use std::io::Read;
use std::{error::Error, path::Path};
use tokio::fs;

#[derive(Default)]
pub struct LocalFilesystem {}

#[async_trait::async_trait(?Send)]
impl Transport for LocalFilesystem {
    async fn read(&mut self, filename: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(fs::read(filename).await?)
    }

    async fn mkdir(&mut self, _path: &Path) -> Result<(), Box<dyn Error>> {
        let t = rand::thread_rng().gen_range(0.0..0.1);
        tokio::time::sleep(std::time::Duration::from_secs_f64(t)).await;
        Ok(())
    }

    async fn write(
        &mut self,
        _filename: &Path,
        _destination: Box<dyn Read + Send>,
    ) -> Result<u64, Box<dyn Error>> {
        let t = rand::thread_rng().gen_range(0.0..0.2);
        // if t > 0.1 {
        //     return Err("timeout".into());
        // }
        tokio::time::sleep(std::time::Duration::from_secs_f64(t)).await;
        Ok((t * 1024.0) as u64)
    }

    async fn remove(&mut self, _pathname: &Path) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
