use std::io::Write;

use std::fs::OpenOptions;
use std::path::PathBuf;

use actix;
use actix_web::error::{BlockingError, ErrorInternalServerError, ErrorUnauthorized, PayloadError};
use actix_web::middleware;
use actix_web::web::HttpResponse;
use actix_web::{App, HttpServer};

use actix_multipart::{Field, Multipart, MultipartError};

use futures::future::{err, Either};
use futures::{Future, Stream};

mod actix_crypt;

use actix_crypt::CryptFiles;
use actix_crypt::EncryptedBlob;

use rand;
use rand::RngCore;

use hex;

use dotenv::dotenv;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref BASE_URL: String = std::env::var("BASE_URL").expect("BASE_URL must be set");
    pub static ref IP: String = std::env::var("IP").expect("IP must be set");
    pub static ref PORT: String = std::env::var("PORT").expect("PORT must be set");
}

pub fn download_file(field: Field) -> impl Future<Item = String, Error = actix_web::error::Error> {
    let mut data = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut data);
    let hex_str = hex::encode(data);

    let mut file_path = std::env::temp_dir();

    let cd = field.content_disposition();
    let filename = if let Some(cd) = cd {
        let filename_opt = cd.get_filename();
        let mut res = hex_str.to_string();

        if let Some(filename) = filename_opt {
            let file_extension_opt = filename.split('.').nth(1);
            if let Some(file_extension) = file_extension_opt {
                res = format!("{}.{}", hex_str, file_extension)
            }
        }
        res
    } else {
        hex_str.to_string()
    };

    file_path.push(filename);

    let mut option = OpenOptions::new();
    option.write(true).read(true).create(true);
    let file = match option.open(file_path.clone()) {
        Ok(file) => file,
        Err(e) => return Either::A(err(ErrorInternalServerError(e))),
    };

    let content_type = field.content_type();
    Either::B(
        field
            .fold(
                (file, file_path, 0i64),
                move |(mut file, file_path, mut acc), bytes| {
                    // fs operations are blocking, we have to execute writes
                    // on threadpool
                    actix_web::web::block(move || {
                        file.write_all(bytes.as_ref()).map_err(|e| {
                            println!("file.write_all failed: {:?}", e);

                            // Try removing the file.
                            std::fs::remove_file(file_path.clone()).ok();
                            MultipartError::Payload(PayloadError::Io(e))
                        })?;
                        acc += bytes.len() as i64;
                        Ok((file, file_path, acc))
                    })
                    .map_err(|e: BlockingError<MultipartError>| match e {
                        BlockingError::Error(e) => e,
                        BlockingError::Canceled => MultipartError::Incomplete,
                    })
                },
            )
            .map(|(file, file_path, _)| {
                let mut encrypted_blob =
                    EncryptedBlob::from(file).map_err(ErrorInternalServerError)?;

                if encrypted_blob.is_header_magic_valid() && encrypted_blob.is_content_valid() {
                    // Valid content, move to bucket

                    let file_name = file_path.file_name().unwrap();

                    let mut bucket_path = PathBuf::from("./bucket");
                    bucket_path.push(file_name);

                    std::fs::copy(file_path.clone(), bucket_path)
                        .map_err(ErrorInternalServerError)?;
                    std::fs::remove_file(file_path.clone())?;
                    return Ok(format!(
                        "{}/{}\n",
                        BASE_URL.as_str(),
                        file_name.to_string_lossy()
                    ));
                }

                Err(ErrorUnauthorized("Authentification failed"))
            })
            .map_err(|e| {
                println!("file download failed, {:?}", e);
                ErrorInternalServerError(e)
            })
            .and_then(|res| res),
    )
}

pub fn upload(multipart: Multipart) -> impl Future<Item = HttpResponse, Error = actix_web::Error> {
    multipart
        .map_err(ErrorInternalServerError)
        .map(|field| download_file(field).into_stream())
        .flatten()
        .collect()
        .map(|res| HttpResponse::Ok().body(res[0].clone()))
        .map_err(|e| {
            println!("failed: {}", e);
            e
        })
}

fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();

    let system = actix::System::new("imagers");

    let bind_string = format!("{}:{}", IP.as_str(), PORT.as_str());

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .service(
                actix_web::web::resource("/upload").route(actix_web::web::post().to_async(upload)),
            )
            .service(CryptFiles::new("/", "./bucket/"))
    })
    .bind(bind_string.as_str())?
    .start();

    system.run()
}
