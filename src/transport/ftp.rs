use super::Transport;
use crate::checksum_tree::ChecksumTree;
use std::io::Read;
use std::net::ToSocketAddrs;
use std::{error::Error, path::Path};
use suppaftp::FtpError;
use suppaftp::FtpStream;

pub struct Connected;
pub struct Disconnected;

pub struct FTP<T = Disconnected> {
    host: String,
    user: String,
    pass: String,
    dir: String,
    stream: Option<FtpStream>,
    _data: std::marker::PhantomData<T>,
}

impl FTP<Disconnected> {
    pub fn new(
        host: impl AsRef<str>,
        user: impl AsRef<str>,
        pass: impl AsRef<str>,
        dir: impl AsRef<str>,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            host: host.as_ref().to_string(),
            user: user.as_ref().to_string(),
            pass: pass.as_ref().to_string(),
            dir: dir.as_ref().to_string(),
            stream: None,
            _data: std::marker::PhantomData,
        })
    }

    pub async fn connect(self) -> Result<FTP<Connected>, Box<dyn Error>> {
        let ip = &self
            .host
            .to_socket_addrs()?
            .find(|addr| addr.is_ipv4())
            .ok_or("Could not resolve host")?;
        let mut stream = FtpStream::connect(ip)?;
        // .into_secure(
        //     TlsConnector::new()?.into(),
        //     "dev.dklab.cz.uvds648.active24.cz",
        // )?;
        stream.login(&self.user, &self.pass)?;
        stream.cwd(&self.dir)?;
        Ok(FTP {
            host: self.host,
            user: self.user,
            pass: self.pass,
            dir: self.dir,
            stream: Some(stream),
            _data: std::marker::PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl Transport for FTP<Connected> {
    async fn get_last_checksum(
        &mut self,
        checksum_filepath: &Path,
    ) -> Result<ChecksumTree, Box<dyn Error>> {
        Ok(self
            .stream
            .as_mut()
            .unwrap()
            .retr_as_buffer(checksum_filepath.to_str().ok_or(format!(
                "Failed converting Path to str: {checksum_filepath:?}"
            ))?)
            .ok()
            .map(|bytes| serde_json::from_slice::<ChecksumTree>(&bytes.into_inner()))
            .transpose()?
            .unwrap_or_default())
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        match self
            .stream
            .as_mut()
            .unwrap()
            .mkdir(path.to_str().ok_or("Fail converting path to str")?)
            .map_err(|e| Box::<dyn Error>::from(format!("Mkdir failed with error: {e}")))
        {
            Err(e) => {
                if e.to_string().contains("File exists") {
                    // safe to ignore
                    return Ok(());
                }
                Err(e)
            }
            x => x,
        }
    }

    async fn upload(
        &mut self,
        filename: &Path,
        mut r: Box<dyn Read + Send>,
    ) -> Result<u64, Box<dyn Error>> {
        self.stream
            .as_mut()
            .unwrap()
            .put_file(
                filename
                    .to_str()
                    .ok_or(format!("Failed converting Path to str: {filename:?}"))
                    .map_err(FtpError::SecureError)?,
                &mut r,
            )
            .map_err(|e| e.into())
    }

    async fn remove(&mut self, mut pathname: &Path) -> Result<(), Box<dyn Error>> {
        self.stream.as_mut().unwrap().rm(pathname
            .to_str()
            .ok_or(format!("Failed converting Path to str: {pathname:?}"))
            .map_err(FtpError::SecureError)?)?;

        while let Some(parent_pathname) = pathname.parent() {
            self.stream
                .as_mut()
                .unwrap()
                .rmdir(
                    parent_pathname
                        .to_str()
                        .ok_or(format!("Failed converting Path to str: {pathname:?}"))
                        .map_err(FtpError::SecureError)?,
                )
                .ok(); // ignore errors about deleting directories
            pathname = parent_pathname;
        }

        Ok(())
    }

    async fn close(mut self: Box<Self>) -> Result<(), Box<dyn Error>> {
        self.stream.as_mut().unwrap().quit().map_err(|e| e.into())
    }
}
