use super::Transport;
use futures::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::io::ErrorKind;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::{error::Error, path::Path};
use suppaftp::async_native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::FtpError;
use suppaftp::FtpStream;
use tokio::io::AsyncRead;
use tokio_util::compat::TokioAsyncReadCompatExt;

pub struct Connected;
pub struct Disconnected;

pub struct Ftp<T = Disconnected> {
    host: String,
    user: String,
    pass: String,
    dir: String,
    stream: Option<FtpStream>,
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
        let domain = self.host.split(':').next().expect("Domain not valid");
        let mut stream = FtpStream::connect(ip).await?;
        if use_tls {
            stream = stream
                .into_secure(
                    TlsConnector::new()
                        .danger_accept_invalid_certs(true)
                        .danger_accept_invalid_hostnames(true)
                        .into(),
                    domain,
                )
                .await?;
        }
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

#[async_trait::async_trait(?Send)]
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
        let writer = self
            .stream
            .as_mut()
            .unwrap()
            .put_with_stream(
                filename
                    .to_str()
                    .ok_or(format!("failed converting Path to str: {filename:?}"))
                    .map_err(FtpError::SecureError)?,
            )
            .await?;

        let reader = Box::into_pin(reader);
        let reader = Box::pin(TokioAsyncReadCompatExt::compat(reader));

        async fn write_inner(
            mut reader: Pin<Box<dyn futures::AsyncRead>>,
            writer: &mut Pin<Box<impl AsyncWrite>>,
        ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
            let mut buf = [0; 8 * 1024];
            let mut len = 0;

            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => {
                        break;
                    }
                    Ok(n) => {
                        len += n;
                        writer.write_all(&buf[..n]).await?;
                        writer.flush().await?;
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => {
                        return Err(e.into());
                    }
                };
            }
            Ok(len as u64)
        }

        let mut writer = Box::pin(writer);
        let result = write_inner(reader, &mut writer).await;
        self.stream
            .as_mut()
            .unwrap()
            .finalize_put_stream(writer)
            .await?;
        result
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
