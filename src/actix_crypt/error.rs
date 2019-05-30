//! based from actix_files
use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use derive_more::Display;

/// Errors which can occur when serving crypt files.
#[derive(Display, Debug, PartialEq)]
pub enum CryptFilesError {
    #[display(fmt = "Nothing to see here")]
    IsDirectory,
}

/// Return `Forbidden` for `CryptFilesError`
impl ResponseError for CryptFilesError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::FORBIDDEN)
    }
}

#[derive(Display, Debug, PartialEq)]
pub enum UriSegmentError {
    /// The segment started with the wrapped invalid character.
    #[display(fmt = "The segment started with the wrapped invalid character")]
    BadStart(char),
    /// The segment contained the wrapped invalid character.
    #[display(fmt = "The segment contained the wrapped invalid character")]
    BadChar(char),
    /// The segment ended with the wrapped invalid character.
    #[display(fmt = "The segment ended with the wrapped invalid character")]
    BadEnd(char),
}

/// Return `BadRequest` for `UriSegmentError`
impl ResponseError for UriSegmentError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST)
    }
}
