use std::pin::Pin;
use tokio::io::AsyncRead;

pub struct ProgressStream<R> {
    reader: R,
    bytes_read: usize,
    update_progress_callback: Box<dyn Fn(u64) + Send>,
}

impl<R> ProgressStream<R> {
    pub fn new(reader: R, update_progress_callback: Box<dyn Fn(u64) + Send>) -> Self {
        Self {
            reader,
            bytes_read: 0,
            update_progress_callback,
        }
    }

    pub fn progress(&self) -> usize {
        self.bytes_read
    }
}

impl<R> AsyncRead for ProgressStream<R>
where
    R: AsyncRead + Unpin + Send,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.reader).poll_read(cx, buf);
        let after = buf.filled().len();
        self.bytes_read += after - before;

        (self.update_progress_callback)(self.bytes_read as u64);

        poll
    }
}
