use super::Transport;
use ssh2::{Session, Sftp};
use std::{
    error::Error,
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::TcpStream,
};

pub struct SFtp {
    session: Session,
    sftp: Sftp,
    dir: String,
}

impl SFtp {
    pub async fn new(
        host: impl AsRef<str>,
        user: impl AsRef<str>,
        pass: impl AsRef<str>,
        dir: impl Into<String>,
    ) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let tcp = TcpStream::connect(host.as_ref()).await?;
        let mut session = Session::new().unwrap();
        session.set_tcp_stream(tcp);
        session.handshake().unwrap();

        session
            .userauth_password(user.as_ref(), pass.as_ref())
            .unwrap();

        let sftp = session.sftp()?;
        let dir = dir.into();
        let dir_path = Path::new(&dir);
        match sftp.readdir(dir_path) {
            Ok(_) => {}
            Err(_) => {
                for part in dir_path
                    .ancestors()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .filter(|part| !part.to_string_lossy().is_empty())
                {
                    sftp.mkdir(part, 0o755)?;
                }
            }
        }

        Ok(Self { session, sftp, dir })
    }

    fn get_path(&self, filename: &Path) -> Result<PathBuf, Box<dyn Error + Send + Sync + 'static>> {
        Ok(PathBuf::from_str(&format!(
            "{dir}/{filename}",
            dir = self.dir,
            filename = filename.display()
        ))?)
    }
}

#[async_trait::async_trait]
impl Transport for SFtp {
    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let mut file = self.sftp.open(self.get_path(filename)?.as_path())?;
        let mut buf = vec![];
        let _ = file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.sftp.mkdir(self.get_path(path)?.as_path(), 0o755)?;
        Ok(())
    }

    async fn write(
        &mut self,
        filename: &Path,
        mut reader: Box<dyn AsyncRead + Unpin + Send>,
        _file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let mut file = self.sftp.create(self.get_path(filename)?.as_path())?;
        let mut buf = vec![0; 1024 * 16]; // 16KB buffer
        let mut read = 0;
        while let Ok(len) = reader.read(&mut buf).await {
            if len == 0 {
                break;
            }
            tokio::task::block_in_place(|| file.write_all(&buf[..len]))?;
            read += len;
        }
        Ok(read as u64)
    }

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut pathname = self.get_path(pathname)?;
        self.sftp.unlink(pathname.as_path())?;

        while let Some(parent_pathname) = pathname.parent() {
            if self.sftp.rmdir(parent_pathname).ok().is_none() {
                // ignore errors about deleting directories but bail out on first error
                break;
            }
            pathname = parent_pathname.to_path_buf();
        }
        Ok(())
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.session.disconnect(None, "close", None)?;
        Ok(())
    }
}
