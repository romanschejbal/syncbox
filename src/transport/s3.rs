use futures::stream::TryStreamExt;
use rusoto_core::{ByteStream, Region};
use rusoto_s3::{DeleteObjectRequest, GetObjectRequest, PutObjectRequest, S3Client, S3};
use std::io::{self, Cursor};
use std::path::PathBuf;
use std::{error::Error, path::Path};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::checksum_tree::ChecksumTree;

use super::Transport;

pub struct AwsS3 {
    bucket: String,
    client: S3Client,
    storage_class: String,
    directory: PathBuf,
}

impl AwsS3 {
    pub fn new(
        bucket: impl AsRef<str>,
        region: impl AsRef<str>,
        access_key: impl AsRef<str>,
        secret_key: impl AsRef<str>,
        storage_class: impl AsRef<str>,
        directory: PathBuf,
    ) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let client = S3Client::new_with(
            rusoto_core::request::HttpClient::new().unwrap(),
            rusoto_credential::StaticProvider::new_minimal(
                access_key.as_ref().to_string(),
                secret_key.as_ref().to_string(),
            ),
            region.as_ref().parse::<Region>()?,
        );
        Ok(Self {
            bucket: bucket.as_ref().to_string(),
            client,
            storage_class: storage_class.as_ref().to_string(),
            directory,
        })
    }

    fn make_object_key(&self, path: &Path) -> String {
        let mut filename_with_prefix = PathBuf::new();
        filename_with_prefix.push(&self.directory);
        let key = filename_with_prefix
            .join(path)
            .components()
            .filter(|c| c.as_os_str() != ".")
            .collect::<PathBuf>()
            .to_string_lossy()
            .to_string();
        key
    }

    async fn write(
        &self,
        file_path: &Path,
        storage_class: String,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let stream =
            FramedRead::new(reader, BytesCodec::new()).map_ok(|bytes_mut| bytes_mut.freeze());

        let file_size_usize = file_size
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "File size is too large"))?;

        let file_stream = ByteStream::new_with_size(stream, file_size_usize);

        let key = self.make_object_key(file_path);

        // Construct the request
        let request = PutObjectRequest {
            bucket: self.bucket.to_string(),
            key,
            body: Some(file_stream),
            storage_class: Some(storage_class),
            ..Default::default()
        };

        // Send the request
        self.client.put_object(request).await.map_err(Box::new)?;

        Ok(file_size)
    }
}

#[async_trait::async_trait(?Send)]
impl Transport for AwsS3 {
    async fn read(
        &mut self,
        filename: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync + 'static>> {
        let key = self.make_object_key(filename);

        // Read file from S3
        let get_req = GetObjectRequest {
            bucket: self.bucket.to_string(),
            key,
            ..Default::default()
        };

        match self.client.get_object(get_req).await {
            Ok(output) => {
                if let Some(stream) = output.body {
                    let mut body = stream.into_async_read();
                    let mut contents = Vec::new();
                    body.read_to_end(&mut contents).await?;
                    Ok(contents)
                } else {
                    Err("No content found in S3 object".into())
                }
            }
            Err(e) => Err(format!("Error getting object: {}", e).into()),
        }
    }

    async fn mkdir(&mut self, _path: &Path) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        // We don't need to create directories in S3
        Ok(())
    }

    async fn write_last_checksum(
        &mut self,
        checksum_filename: &Path,
        checksum_tree: &ChecksumTree,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let json = serde_json::to_string_pretty(checksum_tree)?;
        let file_size = json.len();
        let cursor = Cursor::new(json);
        AwsS3::write(
            self,
            checksum_filename,
            "STANDARD".to_string(),
            Box::new(cursor),
            file_size as u64,
        )
        .await
    }

    async fn write(
        &mut self,
        filename: &Path,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        AwsS3::write(
            self,
            filename,
            self.storage_class.clone(),
            reader,
            file_size,
        )
        .await
    }

    async fn remove(
        &mut self,
        pathname: &Path,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let key = self.make_object_key(pathname);
        let delete_req = DeleteObjectRequest {
            bucket: self.bucket.to_string(),
            key,
            ..Default::default()
        };
        Ok(self.client.delete_object(delete_req).await.map(|_| ())?)
    }

    async fn close(mut self: Box<Self>) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        Ok(())
    }
}
