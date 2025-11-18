//! Retry transport wrapper that provides automatic retry functionality for transport operations.
//!
//! This module provides a `RetryTransport` struct that wraps any transport implementation
//! and automatically retries failed operations using exponential backoff. When an operation
//! fails, the transport connection is dropped and a new one is created for the next attempt.
//!
//! # Example
//!
//! ```rust
//! use syncbox::transport::retry::{RetryTransport, RetryConfig, ConfigBasedTransportFactory};
//! use syncbox::config::Args;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
//! let args = Args::parse(); // Your app args
//! let factory = Arc::new(ConfigBasedTransportFactory::new(args.clone()));
//! let retry_config = RetryConfig::from_args(&args);
//!
//! let mut retry_transport = RetryTransport::new(factory, retry_config);
//!
//! // Operations will automatically retry on failure
//! let data = retry_transport.read(std::path::Path::new("file.txt")).await?;
//! # Ok(())
//! # }
//! ```

use super::Transport;
use crate::config::Args;
use std::{error::Error, path::Path, sync::Arc, time::Duration};
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
    async fn read_last_checksum(
        &mut self,
        checksum_filename: &Path,
    ) -> Result<crate::checksum_tree::ChecksumTree, Box<dyn Error + Send + Sync + 'static>> {
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            // Execute the operation
            if let Some(transport) = &mut self.transport {
                match transport.read_last_checksum(checksum_filename).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }

    async fn write_last_checksum(
        &mut self,
        checksum_filename: &Path,
        checksum_tree: &crate::checksum_tree::ChecksumTree,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            // Execute the operation
            if let Some(transport) = &mut self.transport {
                match transport
                    .write_last_checksum(checksum_filename, checksum_tree)
                    .await
                {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }

    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            // Execute the operation
            if let Some(transport) = &mut self.transport {
                match transport.read(filename).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }

    async fn mkdir(&mut self, path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            // Execute the operation
            if let Some(transport) = &mut self.transport {
                match transport.mkdir(path).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
    }

    async fn write(
        &mut self,
        filename: &Path,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        // For write operations, we can only retry transport creation failures,
        // not write operation failures, because the AsyncRead reader is consumed.
        // This ensures we have a working transport before attempting the write.
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance, with retry on creation failures
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                }
            }

            // Execute the write operation (no retry here due to reader consumption)
            if let Some(transport) = &mut self.transport {
                return match transport.write(filename, reader, file_size).await {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        // Drop the failed transport for future operations
                        self.transport = None;
                        Err(e)
                    }
                };
            }
        }

        Err("Failed to create transport for write operation".into())
    }

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut last_error = None;
        let mut delay = self.config.initial_delay;

        for attempt in 0..=self.config.max_retries {
            // Ensure we have a transport instance
            if self.transport.is_none() {
                match self.factory.create().await {
                    Ok(transport) => self.transport = Some(transport),
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                        continue;
                    }
                }
            }

            // Execute the operation
            if let Some(transport) = &mut self.transport {
                match transport.remove(pathname).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        self.transport = None;
                        if attempt < self.config.max_retries {
                            sleep(delay).await;
                            delay = std::cmp::min(delay * 2, self.config.max_delay);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "All retry attempts failed".into()))
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
