//! Chunked read stream

use actix_web::error::{BlockingError, Error, ErrorInternalServerError};
use bytes::Bytes;
use futures::{Async, Future, Poll, Stream};
use std::any::Any;
use std::io::{Read, Seek, SeekFrom};

const BUFFER_SIZE: u64 = 65_536;

pub struct ChunkedReadStream<T: Read + Seek + Sized + Send + Sync + Any> {
    size: u64,
    offset: u64,
    file: Option<T>,
    fut: Option<Box<Future<Item = (T, Bytes), Error = BlockingError<std::io::Error>>>>,
    counter: u64,
}

fn handle_error(err: BlockingError<std::io::Error>) -> Error {
    match err {
        BlockingError::Error(err) => err.into(),
        BlockingError::Canceled => ErrorInternalServerError("Unexpected error"),
    }
}

impl<T: Read + Seek + Sized + Send + Sync + Any> ChunkedReadStream<T> {
    pub fn new(offset: u64, size: u64, file: T) -> Self {
        ChunkedReadStream {
            offset,
            size,
            file: Some(file),
            fut: None,
            counter: 0,
        }
    }
}

impl<T: Read + Seek + Sized + Send + Sync + Any> Stream for ChunkedReadStream<T> {
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Bytes>, Error> {
        if self.fut.is_some() {
            return match self.fut.as_mut().unwrap().poll().map_err(handle_error)? {
                Async::Ready((file, bytes)) => {
                    self.fut.take();
                    self.file = Some(file);
                    self.offset += bytes.len() as u64;
                    self.counter += bytes.len() as u64;
                    Ok(Async::Ready(Some(bytes)))
                }
                Async::NotReady => Ok(Async::NotReady),
            };
        }

        let size = self.size;
        let offset = self.offset;
        let counter = self.counter;

        if size == counter {
            Ok(Async::Ready(None))
        } else {
            let mut file = self.file.take().expect("Use after completion");
            self.fut = Some(Box::new(actix_web::web::block(move || {
                let max_bytes: usize;
                max_bytes = std::cmp::min(size.saturating_sub(counter), BUFFER_SIZE) as usize;
                let mut buf = Vec::with_capacity(max_bytes);
                file.seek(SeekFrom::Start(offset))?;
                let nbytes = file.by_ref().take(max_bytes as u64).read_to_end(&mut buf);

                if nbytes? == 0 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                Ok((file, Bytes::from(buf)))
            })));
            self.poll()
        }
    }
}
