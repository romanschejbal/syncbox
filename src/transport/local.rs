use super::Transport;
use crate::checksum_tree::ChecksumTree;
use rand::Rng;
use std::io::Read;
use std::{error::Error, path::Path};
use tokio::fs;

#[derive(Default)]
pub struct LocalFilesystem {}

#[async_trait::async_trait]
impl Transport for LocalFilesystem {
    async fn get_last_checksum(
        &mut self,
        checksum_filepath: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error>> {
        fs::read(checksum_filepath)
            .await
            .map(|content| serde_json::from_slice(&content))
            .unwrap_or_else(|_| Ok(ChecksumTree::default()))
            .map_err(|e| e.into())
    }

    async fn mkdir(&mut self, _path: &Path) -> Result<(), Box<dyn Error>> {
        let t = rand::thread_rng().gen_range(0.0..0.1);
        tokio::time::sleep(std::time::Duration::from_secs_f64(t)).await;
        Ok(())
    }

    async fn upload(
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
