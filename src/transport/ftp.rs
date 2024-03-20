use super::Transport;
use futures::AsyncReadExt;
use std::net::ToSocketAddrs;
use std::{error::Error, path::Path};
use suppaftp::async_native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::AsyncNativeTlsConnector;
use suppaftp::{AsyncNativeTlsFtpStream, FtpError};
use tokio::io::AsyncRead;
use tokio_util::compat::TokioAsyncReadCompatExt;

pub struct Connected;
pub struct Disconnected;

pub struct Ftp<T = Disconnected> {
    host: String,
    user: String,
    pass: String,
    dir: String,
    stream: Option<AsyncNativeTlsFtpStream>,
    _data: std::marker::PhantomData<T>,
}

impl Ftp<Disconnected> {
    pub fn new(
        host: impl AsRef<str>,
        user: impl AsRef<str>,
        pass: impl AsRef<str>,
        dir: impl AsRef<str>,
    ) -> Self {
        Self {
            host: host.as_ref().to_string(),
            user: user.as_ref().to_string(),
            pass: pass.as_ref().to_string(),
            dir: dir.as_ref().to_string(),
            stream: None,
            _data: std::marker::PhantomData,
        }
    }

    pub async fn connect(
        self,
        use_tls: bool,
    ) -> Result<Ftp<Connected>, Box<dyn Error + Send + Sync + 'static>> {
        let ip = &self
            .host
            .to_socket_addrs()?
            .find(|addr| addr.is_ipv4())
            .ok_or("could not resolve host")?;
        let domain = self
            .host
            .split(':')
            .next()
            .expect("domain not valid, should be in form ip:port");
        let mut stream = AsyncNativeTlsFtpStream::connect(ip).await?;
        if use_tls {
            let connector = TlsConnector::new()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
            stream = stream
                .into_secure(AsyncNativeTlsConnector::from(connector), domain)
                .await?;
        }
        stream.set_mode(suppaftp::Mode::ExtendedPassive);
        stream.login(&self.user, &self.pass).await?;
        stream.cwd(&self.dir).await?;
        Ok(Ftp {
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
impl Transport for Ftp<Connected> {
    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let mut buf = vec![];
        self.stream
            .as_mut()
            .unwrap()
            .transfer_type(FileType::Binary)
            .await?;
        let mut stream = self
            .stream
            .as_mut()
            .unwrap()
            .retr_as_stream(
                filename
                    .to_str()
                    .ok_or(format!("failed converting Path to str: {filename:?}"))?,
            )
            .await?;
        stream.read_to_end(&mut buf).await?;
        self.stream
            .as_mut()
            .unwrap()
            .finalize_retr_stream(stream)
            .await?;
        Ok(buf)
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        match self
            .stream
            .as_mut()
            .unwrap()
            .mkdir(path.to_str().ok_or("fail converting path to str")?)
            .await
            .map_err(|e| {
                Box::<dyn Error + Send + Sync + 'static>::from(format!(
                    "mkdir failed with error: {e}"
                ))
            }) {
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

    async fn write(
        &mut self,
        filename: &Path,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        _file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        self.stream
            .as_mut()
            .unwrap()
            .transfer_type(FileType::Binary)
            .await?;
        let size = self
            .stream
            .as_mut()
            .unwrap()
            .put_file(
                filename.to_str().ok_or(format!(
                    "failed converting path to str, filename: {filename:?}"
                ))?,
                &mut reader.compat(),
            )
            .await?;
        Ok(size)
    }

    async fn remove(
        &mut self,
        mut pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.stream
            .as_mut()
            .unwrap()
            .rm(pathname
                .to_str()
                .ok_or(format!("failed converting Path to str: {pathname:?}"))
                .map_err(FtpError::SecureError)?)
            .await?;

        while let Some(parent_pathname) = pathname.parent() {
            if self
                .stream
                .as_mut()
                .unwrap()
                .rmdir(
                    parent_pathname
                        .to_str()
                        .ok_or(format!("failed converting Path to str: {pathname:?}"))
                        .map_err(FtpError::SecureError)?,
                )
                .await
                .ok()
                .is_none()
            {
                // ignore errors about deleting directories but bail out on first error
                break;
            }
            pathname = parent_pathname;
        }

        Ok(())
    }

    async fn close(mut self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(self.stream.as_mut().unwrap().quit().await?)
    }
}
