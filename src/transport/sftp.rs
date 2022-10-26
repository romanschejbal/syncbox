use crate::checksum_tree::ChecksumTree;
use crate::transport::Transport;
use openssh::{KnownHosts, Session, Stdio};
use openssh_sftp_client::Sftp as SftpClient;
use std::{error::Error, io::Read, marker::PhantomData, net::ToSocketAddrs, path::Path, sync::Arc};
use tokio::sync::Mutex;

pub struct Connected;
pub struct Disconnected;

pub struct Sftp<T = Disconnected> {
    host: String,
    user: String,
    pass: String,
    dir: String,
    client: Arc<Mutex<Option<SftpClient>>>,
    _data: std::marker::PhantomData<T>,
}

impl Sftp<Disconnected> {
    pub fn new(host: String, user: String, pass: String, dir: String) -> Self {
        Self {
            host,
            user,
            pass,
            dir,
            client: Arc::new(Mutex::new(None)),
            _data: PhantomData,
        }
    }

    pub async fn connect(self) -> Result<Sftp<Connected>, Box<dyn Error>> {
        let ip = &self
            .host
            .to_socket_addrs()?
            .find(|addr| addr.is_ipv4())
            .ok_or("Could not resolve host")?;
        println!("ssh://{}@{}", self.user, ip.to_string());
        let session = Session::connect(
            format!("ssh://{}@{}", self.user, ip.to_string()),
            KnownHosts::Accept,
        )
        .await?;
        let mut child = session
            .subsystem("sftp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .await?;

        let client = SftpClient::new(
            child.stdin().take().unwrap(),
            child.stdout().take().unwrap(),
            Default::default(),
        )
        .await?;
        Ok(Sftp {
            host: self.host,
            user: self.user,
            pass: self.pass,
            dir: self.dir,
            client: Arc::new(Mutex::new(Some(client))),
            _data: PhantomData,
        })
    }
}

#[async_trait::async_trait(?Send)]
impl Transport for Sftp<Connected> {
    async fn read(&mut self, filename: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut client = self.client.lock().await;
        Ok(client
            .as_mut()
            .unwrap()
            .fs()
            .read(filename)
            .await
            .map(|bytes| bytes.to_vec())?)
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        // todo try 'static on the future only
        unimplemented!()
    }

    async fn write(
        &mut self,
        filename: &Path,
        read: Box<dyn Read + Send>,
    ) -> Result<u64, Box<dyn Error>> {
        unimplemented!()
    }

    async fn remove(&mut self, pathname: &Path) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }

    async fn close(self: Box<Self>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
}
