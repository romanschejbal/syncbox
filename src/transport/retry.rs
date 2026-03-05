//! Retry transport wrapper that provides automatic retry functionality for transport operations.
//!
//! This module provides a `RetryTransport` struct that wraps any transport implementation
//! and automatically retries failed operations using exponential backoff. When an operation
//! fails, the transport connection is dropped and a new one is created for the next attempt.

use super::Transport;
use crate::config::Args;
use rand::Rng;
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
