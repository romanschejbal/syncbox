//! Retry transport wrapper that provides automatic retry functionality for transport operations.
//!
//! This module provides a `RetryTransport` struct that wraps any transport implementation
//! and automatically retries failed operations using exponential backoff. When an operation
//! fails, the transport connection is dropped and a new one is created for the next attempt.

use super::Transport;
use crate::checksum_tree::ChecksumTree;
use crate::config::Args;
use rand::Rng;
use std::{error::Error, io::Cursor, path::Path, sync::Arc, time::Duration};
use tokio::{io::AsyncRead, time::sleep};

/// A factory trait for creating transport instances
#[async_trait::async_trait]
pub trait TransportFactory: Send + Sync {
    async fn create(
        &self,
    ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>>;
}

/// Configuration for retry behavior
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_retries: usize,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl RetryConfig {
    pub fn new(max_retries: usize, initial_delay_ms: u64, max_delay_secs: u64) -> Self {
        Self {
            max_retries,
            initial_delay: Duration::from_millis(initial_delay_ms),
            max_delay: Duration::from_secs(max_delay_secs),
        }
    }

    pub fn from_args(args: &Args) -> Self {
        Self::new(
            args.max_retries,
            args.initial_retry_delay,
            args.max_retry_delay,
        )
    }
}

fn delay_with_jitter(delay: Duration) -> Duration {
    let jitter_ms = delay.as_millis() / 4;
    if jitter_ms == 0 {
        return delay;
    }
    let jitter = rand::thread_rng().gen_range(0..=jitter_ms) as u64;
    delay + Duration::from_millis(jitter)
}

/// Macro to deduplicate retry loop logic for retryable operations.
macro_rules! retry_op {
    ($self:ident, |$transport:ident| $op:expr) => {{
        let mut last_error = None;
        let mut delay = $self.config.initial_delay;

        for attempt in 0..=$self.config.max_retries {
            if $self.transport.is_none() {
                match $self.factory.create().await {
                    Ok(transport) => $self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < $self.config.max_retries {
                            sleep(delay_with_jitter(delay)).await;
                            delay = std::cmp::min(delay * 2, $self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            if let Some($transport) = &mut $self.transport {
                match $op.await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        $self.transport = None;
                        if attempt < $self.config.max_retries {
                            sleep(delay_with_jitter(delay)).await;
                            delay = std::cmp::min(delay * 2, $self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }};
}

/// A transport wrapper that provides automatic retry functionality with exponential backoff
pub struct RetryTransport {
    factory: Arc<dyn TransportFactory>,
    transport: Option<Box<dyn Transport + Send + Sync>>,
    config: RetryConfig,
}

impl RetryTransport {
    pub fn new(factory: Arc<dyn TransportFactory>, config: RetryConfig) -> Self {
        Self {
            factory,
            transport: None,
            config,
        }
    }
}

#[async_trait::async_trait]
impl Transport for RetryTransport {
    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        retry_op!(self, |transport| transport.read(filename))
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        retry_op!(self, |transport| transport.mkdir(path))
    }

    async fn write(
        &mut self,
        filename: &Path,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        // For write operations, we can only retry transport creation failures,
        // not write operation failures, because the AsyncRead reader is consumed.
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        if attempt < self.config.max_retries {
                            sleep(delay_with_jitter(delay)).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                }
            }

            if let Some(transport) = &mut self.transport {
                return match transport.write(filename, reader, file_size).await {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        self.transport = None;
                        Err(e)
                    }
                };
            }
        }

        Err("Failed to create transport for write operation".into())
    }

    async fn write_last_checksum(
        &mut self,
        checksum_filename: &Path,
        checksum_tree: &ChecksumTree,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        // Unlike generic write(), the checksum data can be recreated on each attempt,
        // so we can fully retry both transport creation and the write operation.
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay_with_jitter(delay)).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            if let Some(transport) = &mut self.transport {
                let json = checksum_tree.to_gzip()?;
                let file_size = json.len() as u64;
                let cursor = Cursor::new(json);
                match transport
                    .write(checksum_filename, Box::new(cursor), file_size)
                    .await
                {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay_with_jitter(delay)).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed for checksum upload".into()))
    }

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        retry_op!(self, |transport| transport.remove(pathname))
    }

    async fn close(mut self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        if let Some(transport) = self.transport.take() {
            transport.close().await
        } else {
            Ok(())
        }
    }
}

/// A concrete implementation of TransportFactory that holds the configuration
/// and can recreate transports based on the transport type
pub struct ConfigBasedTransportFactory {
    args: Args,
}

impl ConfigBasedTransportFactory {
    pub fn new(args: Args) -> Self {
        Self { args }
    }
}

#[async_trait::async_trait]
impl TransportFactory for ConfigBasedTransportFactory {
    async fn create(
        &self,
    ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>> {
        use crate::config::TransportType;
        use crate::transport::{
            dry::DryTransport, ftp::Ftp, local::LocalFilesystem, s3::AwsS3, sftp::SFtp,
        };

        Ok(match &self.args.transport {
            TransportType::Ftp {
                ftp_host,
                ftp_user,
                ftp_pass,
                ftp_dir,
                use_tls,
            } => Box::new(
                Ftp::new(ftp_host, ftp_user, ftp_pass, ftp_dir)
                    .connect(*use_tls)
                    .await?,
            ),
            TransportType::Sftp {
                host,
                user,
                pass,
                dir,
            } => Box::new(SFtp::new(host, user, pass, dir).await?),
            TransportType::Local { destination } => Box::new(LocalFilesystem::new(destination)),
            TransportType::S3 {
                bucket,
                region,
                access_key,
                secret_key,
                storage_class,
                directory,
            } => Box::new(AwsS3::new(
                bucket,
                region,
                access_key,
                secret_key,
                storage_class,
                directory.into(),
            )?),
            TransportType::Dry => Box::new(DryTransport),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockTransport;

    #[async_trait::async_trait]
    impl Transport for MockTransport {
        async fn read(
            &mut self,
            _filename: &Path,
        ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
            Ok(vec![1, 2, 3])
        }
        async fn mkdir(
            &mut self,
            _path: &Path,
        ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
            Ok(())
        }
        async fn write(
            &mut self,
            _filename: &Path,
            _reader: Box<dyn AsyncRead + Unpin + Send>,
            _file_size: u64,
        ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
            Ok(0)
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

    struct MockFactory {
        fail_count: AtomicUsize,
        max_failures: usize,
    }

    impl MockFactory {
        fn new(max_failures: usize) -> Self {
            Self {
                fail_count: AtomicUsize::new(0),
                max_failures,
            }
        }
    }

    #[async_trait::async_trait]
    impl TransportFactory for MockFactory {
        async fn create(
            &self,
        ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>>
        {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.max_failures {
                Err(format!("mock failure {}", count).into())
            } else {
                Ok(Box::new(MockTransport))
            }
        }
    }

    #[tokio::test]
    async fn retry_succeeds_after_failures() {
        let factory = Arc::new(MockFactory::new(2));
        let config = RetryConfig::new(3, 1, 1);
        let mut transport = RetryTransport::new(factory, config);

        let result = transport.read(Path::new("test.txt")).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn retry_exhausted_returns_error() {
        let factory = Arc::new(MockFactory::new(100)); // always fail
        let config = RetryConfig::new(2, 1, 1);
        let mut transport = RetryTransport::new(factory, config);

        let result = transport.read(Path::new("test.txt")).await;
        assert!(result.is_err());
    }

    #[test]
    fn retry_config_construction() {
        let config = RetryConfig::new(5, 100, 30);
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_millis(100));
        assert_eq!(config.max_delay, Duration::from_secs(30));
    }

    /// A mock transport whose write() fails a configurable number of times before succeeding.
    struct FailingWriteTransport {
        write_fail_count: Arc<AtomicUsize>,
        write_max_failures: usize,
    }

    #[async_trait::async_trait]
    impl Transport for FailingWriteTransport {
        async fn read(
            &mut self,
            _filename: &Path,
        ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
            Ok(vec![])
        }
        async fn mkdir(
            &mut self,
            _path: &Path,
        ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
            Ok(())
        }
        async fn write(
            &mut self,
            _filename: &Path,
            _reader: Box<dyn AsyncRead + Unpin + Send>,
            _file_size: u64,
        ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
            let count = self.write_fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.write_max_failures {
                Err(format!("write failure {}", count).into())
            } else {
                Ok(_file_size)
            }
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

    struct FailingWriteFactory {
        write_fail_count: Arc<AtomicUsize>,
        write_max_failures: usize,
    }

    impl FailingWriteFactory {
        fn new(write_max_failures: usize) -> Self {
            Self {
                write_fail_count: Arc::new(AtomicUsize::new(0)),
                write_max_failures,
            }
        }
    }

    #[async_trait::async_trait]
    impl TransportFactory for FailingWriteFactory {
        async fn create(
            &self,
        ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>>
        {
            Ok(Box::new(FailingWriteTransport {
                write_fail_count: self.write_fail_count.clone(),
                write_max_failures: self.write_max_failures,
            }))
        }
    }

    #[tokio::test]
    async fn write_last_checksum_retries_on_write_failure() {
        let factory = Arc::new(FailingWriteFactory::new(2));
        let config = RetryConfig::new(3, 1, 1);
        let mut transport = RetryTransport::new(factory, config);
        let checksum_tree = ChecksumTree::default();

        let result = transport
            .write_last_checksum(Path::new("checksum.gz"), &checksum_tree)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn write_last_checksum_exhausted_returns_error() {
        let factory = Arc::new(FailingWriteFactory::new(100)); // always fail
        let config = RetryConfig::new(2, 1, 1);
        let mut transport = RetryTransport::new(factory, config);
        let checksum_tree = ChecksumTree::default();

        let result = transport
            .write_last_checksum(Path::new("checksum.gz"), &checksum_tree)
            .await;
        assert!(result.is_err());
    }
}
