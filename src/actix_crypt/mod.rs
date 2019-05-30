//! Custom actix_files that actually work the way I need it.
use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

mod chunked_stream;
mod crypt;
mod error;
mod file;

pub use crypt::EncryptedBlob;

use actix_service::boxed::{BoxedNewService, BoxedService};
use actix_service::{NewService, Service};
use actix_web::dev::*;
use actix_web::error::Error;
use actix_web::{FromRequest, HttpRequest, Responder};

use file::ChunkedCryptFile;
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll};

use error::*;

type HttpService = BoxedService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxedNewService<(), ServiceRequest, ServiceResponse, Error, ()>;

pub struct CryptFiles {
    path: String,
    directory: PathBuf,
    index: Option<String>,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
}

pub struct CryptFilesService {
    directory: PathBuf,
    index: Option<String>,
    default: Option<HttpService>,
}

impl HttpServiceFactory for CryptFiles {
    fn register(self, config: &mut AppService) {
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }
        let rdef = if config.is_root() {
            ResourceDef::root_prefix(&self.path)
        } else {
            ResourceDef::prefix(&self.path)
        };

        config.register_service(rdef, None, self, None)
    }
}

impl NewService for CryptFiles {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Service = CryptFilesService;
    type InitError = ();
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        let mut srv = CryptFilesService {
            directory: self.directory.clone(),
            index: self.index.clone(),
            default: None,
        };

        if let Some(ref default) = *self.default.borrow() {
            Box::new(
                default
                    .new_service(&())
                    .map(move |default| {
                        srv.default = Some(default);
                        srv
                    })
                    .map_err(|_| ()),
            )
        } else {
            Box::new(ok(srv))
        }
    }
}

impl CryptFiles {
    pub fn new<T: Into<PathBuf>>(path: &str, dir: T) -> Self {
        let dir = dir.into().canonicalize().unwrap_or_else(|_| PathBuf::new());
        if !dir.is_dir() {
            //log::error!("Specified path is not a directory");
        }

        CryptFiles {
            path: path.to_string(),
            directory: dir,
            index: None,
            default: Rc::new(RefCell::new(None)),
        }
    }
}

impl CryptFilesService {
    fn handle_err(
        &mut self,
        e: io::Error,
        req: ServiceRequest,
    ) -> Either<
        FutureResult<ServiceResponse, Error>,
        Box<Future<Item = ServiceResponse, Error = Error>>,
    > {
        log::debug!("Files: Failed to handle {}: {}", req.path(), e);
        if let Some(ref mut default) = self.default {
            default.call(req)
        } else {
            Either::A(ok(req.error_response(e)))
        }
    }
}

impl Service for CryptFilesService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        Box<Future<Item = Self::Response, Error = Self::Error>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        // let (req, pl) = req.into_parts();

        let real_path = match PathBufWrp::get_pathbuf(req.match_info().path()) {
            Ok(item) => item,
            Err(e) => return Either::A(ok(req.error_response(e))),
        };

        // full filepath
        let path = match self.directory.join(&real_path.0).canonicalize() {
            Ok(path) => path,
            Err(e) => return self.handle_err(e, req),
        };

        if path.is_dir() {
            if let Some(ref redir_index) = self.index {
                let path = path.join(redir_index);

                match ChunkedCryptFile::open(path) {
                    Ok(crypt_file) => {
                        let (req, _) = req.into_parts();
                        Either::A(ok(match crypt_file.respond_to(&req) {
                            Ok(item) => ServiceResponse::new(req, item),
                            Err(e) => ServiceResponse::from_err(e, req),
                        }))
                    }
                    Err(e) => return self.handle_err(e, req),
                }
            } else {
                Either::A(ok(ServiceResponse::from_err(
                    CryptFilesError::IsDirectory,
                    req.into_parts().0,
                )))
            }
        } else {
            match ChunkedCryptFile::open(path) {
                Ok(crypt_file) => {
                    let (req, _) = req.into_parts();
                    match crypt_file.respond_to(&req) {
                        Ok(item) => Either::A(ok(ServiceResponse::new(req.clone(), item))),
                        Err(e) => Either::A(ok(ServiceResponse::from_err(e, req))),
                    }
                }
                Err(e) => self.handle_err(e, req),
            }
        }
    }
}

#[derive(Debug)]
struct PathBufWrp(PathBuf);

impl PathBufWrp {
    fn get_pathbuf(path: &str) -> Result<Self, UriSegmentError> {
        let mut buf = PathBuf::new();
        for segment in path.split('/') {
            if segment == ".." {
                buf.pop();
            } else if segment.starts_with('.') {
                return Err(UriSegmentError::BadStart('.'));
            } else if segment.starts_with('*') {
                return Err(UriSegmentError::BadStart('*'));
            } else if segment.ends_with(':') {
                return Err(UriSegmentError::BadEnd(':'));
            } else if segment.ends_with('>') {
                return Err(UriSegmentError::BadEnd('>'));
            } else if segment.ends_with('<') {
                return Err(UriSegmentError::BadEnd('<'));
            } else if segment.is_empty() {
                continue;
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::BadChar('\\'));
            } else {
                buf.push(segment)
            }
        }

        Ok(PathBufWrp(buf))
    }
}

impl FromRequest for PathBufWrp {
    type Error = UriSegmentError;
    type Future = Result<Self, Self::Error>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        PathBufWrp::get_pathbuf(req.match_info().path())
    }
}
