use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

use mime;
use mime_guess::guess_mime_type;

use actix_web::http::header::{self, ContentDisposition, DispositionParam, DispositionType};
use actix_web::http::{ContentEncoding, Method, StatusCode};
use actix_web::middleware::BodyEncoding;
use actix_web::{Error, HttpRequest, HttpResponse, Responder};

use actix_files::HttpRange;

use super::crypt::EncryptedBlob;

use super::chunked_stream::ChunkedReadStream;

/// A file to decrypt with a name.
#[derive(Debug)]
pub struct ChunkedCryptFile {
    file: File,
    content_type: mime::Mime,
    content_disposition: header::ContentDisposition,
    file_length: u64,
    encoding: Option<ContentEncoding>,
}

impl ChunkedCryptFile {
    pub fn from_file<P: AsRef<Path>>(file: File, path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        let (content_type, content_disposition) = {
            let filename = match path.file_name() {
                Some(name) => name.to_string_lossy(),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Provided path has no filename",
                    ));
                }
            };

            let ct = guess_mime_type(&path);
            let disposition_type = match ct.type_() {
                mime::IMAGE | mime::TEXT | mime::VIDEO => DispositionType::Inline,
                _ => DispositionType::Attachment,
            };
            let cd = ContentDisposition {
                disposition: disposition_type,
                parameters: vec![DispositionParam::Filename(filename.into_owned())],
            };
            (ct, cd)
        };

        let md = file.metadata()?;
        let encoding = None;
        Ok(ChunkedCryptFile {
            file,
            content_type,
            content_disposition,
            file_length: md.len(),
            encoding,
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut option = OpenOptions::new();
        option.write(true).read(true);
        Self::from_file(option.open(&path)?, path)
    }
}

impl Responder for ChunkedCryptFile {
    type Error = Error;
    type Future = Result<HttpResponse, Error>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match *req.method() {
            Method::HEAD | Method::GET => (),
            _ => {
                return Ok(HttpResponse::MethodNotAllowed()
                    .header(header::CONTENT_TYPE, "text/plain")
                    .header(header::ALLOW, "GET, HEAD")
                    .body("This resource only supports GET and HEAD."));
            }
        }

        let mut resp = HttpResponse::build(StatusCode::OK);
        resp.set(header::ContentType(self.content_type.clone()))
            .header(
                header::CONTENT_DISPOSITION,
                self.content_disposition.to_string(),
            );

        let mut encrypted_file = EncryptedBlob::from(self.file)?;

        let encrypted_is_valid = encrypted_file.is_header_magic_valid();

        let unpadded_size = if encrypted_is_valid {
            Some(encrypted_file.get_unpadded_size()?)
        } else {
            None
        };

        let file = encrypted_file.into_inner();

        // default compressing
        if let Some(current_encoding) = self.encoding {
            resp.encoding(current_encoding);
        }

        resp.header(header::ACCEPT_RANGES, "bytes");

        let file_length = if encrypted_is_valid {
            unpadded_size.unwrap()
        } else {
            self.file_length
        };


        let mut length = file_length;

        let mut offset = 0;

        // check for range header
        if let Some(ranges) = req.headers().get(header::RANGE) {
            if let Ok(rangesheader) = ranges.to_str() {
                if let Ok(rangesvec) = HttpRange::parse(rangesheader, length) {
                    length = rangesvec[0].length;
                    offset = rangesvec[0].start;
                    resp.encoding(ContentEncoding::Identity);
                    resp.header(
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", offset, offset + length - 1, file_length),
                    );
                } else {
                    resp.header(header::CONTENT_RANGE, format!("bytes */{}", length));
                    return Ok(resp.status(StatusCode::RANGE_NOT_SATISFIABLE).finish());
                };
            } else {
                return Ok(resp.status(StatusCode::BAD_REQUEST).finish());
            };
        };

        resp.header(header::CONTENT_LENGTH, format!("{}", length));

        if *req.method() == Method::HEAD {
            Ok(resp.finish())
        } else if encrypted_is_valid {
                let reader = ChunkedReadStream::new(offset, length, EncryptedBlob::from(file)?);
                if offset != 0 || length != file_length {
                    return Ok(resp.status(StatusCode::PARTIAL_CONTENT).streaming(reader));
                };

                Ok(resp.streaming(reader))
        } else {
            let reader = ChunkedReadStream::new(offset, length, file);
            if offset != 0 || length != file_length {
                return Ok(resp.status(StatusCode::PARTIAL_CONTENT).streaming(reader));
            };

            Ok(resp.streaming(reader))
        }
    }
}
