use futures::stream::TryStreamExt;
use rusoto_core::{ByteStream, Region};
use rusoto_s3::{
    CompleteMultipartUploadRequest, CompletedMultipartUpload, CompletedPart,
    CreateMultipartUploadRequest, DeleteObjectRequest, GetObjectRequest,
    ListMultipartUploadsRequest, ListPartsRequest, PutObjectRequest, S3Client, UploadPartRequest,
    S3,
};
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
        mut reader: Box<dyn AsyncRead + Unpin + Send>,
        file_size: u64,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let file_size_usize = file_size
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "File size is too large"))?;

        let key = self.make_object_key(file_path);

        // Use multipart for larger files
        const FILE_SIZE_THRESHOLD: usize = 1024 * 1024 * 100; // 1024 * 1024 * 1024 * 5;
        if file_size_usize > FILE_SIZE_THRESHOLD {
            // divide file to 100MB chunks
            let mut chunk_size: usize = 100 * 1024 * 1024;
            // make sure all chunks will be at least 5MBs
            loop {
                if chunk_size == 0 {
                    return Err("Last chunk is too small".into());
                }
                if file_size_usize % chunk_size < 5 * 1024 * 1024 {
                    chunk_size -= 1;
                } else {
                    break;
                }
            }

            let mut parts = Vec::new();
            let mut part_number = 1;
            let mut buf = vec![0u8; chunk_size];
            let mut read_last = 0;
            let mut max_part_uploaded = 0;

            let multipart_uploads = self
                .client
                .list_multipart_uploads(ListMultipartUploadsRequest {
                    bucket: self.bucket.to_string(),
                    prefix: Some(key.clone()),
                    ..Default::default()
                })
                .await?;

            let mut uploads = multipart_uploads.uploads.unwrap_or(vec![]);
            uploads.sort_by(|a, b| {
                a.initiated
                    .as_ref()
                    .unwrap()
                    .cmp(b.initiated.as_ref().unwrap())
            });

            let upload_id = if let Some(upload_id) = uploads.pop().and_then(|u| u.upload_id) {
                let uploaded_parts = self
                    .client
                    .list_parts(ListPartsRequest {
                        bucket: self.bucket.to_string(),
                        key: key.clone(),
                        upload_id: upload_id.clone(),
                        ..Default::default()
                    })
                    .await?;
                max_part_uploaded = uploaded_parts
                    .parts
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|p| p.part_number.unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                for part in uploaded_parts.parts.unwrap_or(vec![]) {
                    parts.push(CompletedPart {
                        e_tag: part.e_tag,
                        part_number: part.part_number,
                    });
                }
                upload_id
            } else {
                let start_req = self
                    .client
                    .create_multipart_upload(CreateMultipartUploadRequest {
                        bucket: self.bucket.to_string(),
                        key: key.clone(),
                        storage_class: Some(storage_class),
                        ..Default::default()
                    })
                    .await?;
                start_req.upload_id.ok_or("No upload ID received")?
            };

            loop {
                let read = reader.read(&mut buf[read_last..chunk_size]).await?;
                read_last += read;
                if read == 0 && read_last == 0 {
                    break;
                } else if read > 0 && read_last < chunk_size {
                    continue;
                }

                if part_number > max_part_uploaded {
                    let upload_part_req = UploadPartRequest {
                        bucket: self.bucket.to_string(),
                        key: key.clone(),
                        upload_id: upload_id.clone(),
                        part_number,
                        body: Some(buf[..read_last].to_vec().into()),
                        ..Default::default()
                    };
                    let upload_part_res = self.client.upload_part(upload_part_req).await?;

                    let etag = upload_part_res.e_tag.ok_or("No ETag received")?;
                    parts.push(CompletedPart {
                        e_tag: Some(etag),
                        part_number: Some(part_number),
                    });
                }

                part_number += 1;
                read_last = 0;
            }

            let complete_req = CompleteMultipartUploadRequest {
                bucket: self.bucket.to_string(),
                key,
                upload_id,
                multipart_upload: Some(CompletedMultipartUpload { parts: Some(parts) }),
                ..Default::default()
            };
            self.client.complete_multipart_upload(complete_req).await?;

            Ok(file_size)
        } else {
            let stream =
                FramedRead::new(reader, BytesCodec::new()).map_ok(|bytes_mut| bytes_mut.freeze());

            let file_stream = ByteStream::new_with_size(stream, file_size_usize);

            // Construct the request
            let request = PutObjectRequest {
                bucket: self.bucket.to_string(),
                key,
                body: Some(file_stream),
                storage_class: Some(storage_class),
                ..Default::default()
            };

            // Send the request
            self.client.put_object(request).await?;

            Ok(file_size)
        }
    }
}

#[async_trait::async_trait]
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
        let json = checksum_tree.to_gzip()?;
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
