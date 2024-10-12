mod util;

use std::{
    future::Future,
    io::{self},
    pin::Pin,
    sync::Arc,
};

pub use macros::{register_impl, TCPShare};
use serde::{Deserialize, Serialize};
#[cfg(not(feature = "async-tcp"))]
pub use std::net::TcpListener;
#[cfg(not(feature = "async-tcp"))]
use std::{
    io::{Read as _, Write as _},
    net::TcpStream,
};
#[cfg(feature = "async-tcp")]
pub use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
#[cfg(feature = "async-tcp")]
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};
use util::{take_status_code, take_str};

#[derive(thiserror::Error, Debug, Deserialize, Serialize)]
pub enum Error {
    #[error("Buffer too short")]
    BufferTooShort,
    #[error("unknown function")]
    FunctionNotFound,
    #[error("failed to convert bytes to string")]
    Utf8Error,
    #[error("?")]
    StreamError(StreamError),
    #[error("todo: remove later")]
    Custom(String),
    #[error("does not match struct")]
    ApiMisMatch(String),
}

#[derive(Deserialize, Serialize, Debug)]
pub struct StreamError {
    pub code: Option<i32>,
    pub kind: String,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        let kind = err.kind().to_string();
        let code = err.raw_os_error();
        Error::StreamError(StreamError { code, kind })
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn encode<T: Serialize>(data: T) -> Result<Vec<u8>> {
    Ok(bincode::serialize(&data).unwrap())
}

pub fn decode<'a, T>(v: &'a [u8]) -> Result<T>
where
    T: serde::de::Deserialize<'a>,
{
    Ok(bincode::deserialize(v).unwrap())
}

#[cfg(feature = "async-tcp")]
pub async fn send_data(
    port: u16,
    magic_header: &str,
    func: &str,
    data: Vec<u8>,
) -> Result<Vec<u8>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let header = magic_header.as_bytes();
    let func = func.as_bytes();
    let mut buffer = vec![];
    buffer.extend((header.len() as u32).to_ne_bytes());
    buffer.extend(header);
    buffer.extend((func.len() as u32).to_ne_bytes());
    buffer.extend(func);
    buffer.extend(data);
    let length = buffer.len() as u32;
    let mut response = vec![];
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(&buffer).await?;
    stream.read_to_end(&mut response).await?;
    let mut response: &[u8] = &response;
    let status = take_status_code(&mut response)?;
    if status == 0 {
        Ok(response.to_vec())
    } else {
        let err: Result<Error> = decode(response);
        match err {
            Ok(err) => Err(err),
            Err(err) => Err(err),
        }
    }
}

#[cfg(not(feature = "async-tcp"))]
pub fn send_data(port: u16, magic_header: &str, func: &str, data: Vec<u8>) -> Result<Vec<u8>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    let header = magic_header.as_bytes();
    let func = func.as_bytes();
    let mut buffer = vec![];
    buffer.extend((header.len() as u32).to_ne_bytes());
    buffer.extend(header);
    buffer.extend((func.len() as u32).to_ne_bytes());
    buffer.extend(func);
    buffer.extend(data);
    let length = buffer.len() as u32;
    let mut response = vec![];

    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&buffer)?;
    stream.read_to_end(&mut response)?;
    let mut response: &[u8] = &response;
    let status = take_status_code(&mut response)?;
    if status == 0 {
        Ok(response.to_vec())
    } else {
        let err: Result<Error> = decode(response);
        match err {
            Ok(err) => Err(err),
            Err(err) => Err(err),
        }
    }
}

#[cfg(feature = "async-tcp")]
async fn receive_data(
    stream: &mut TcpStream,
    magic_header_server: &str,
) -> Result<(String, Vec<u8>)> {
    let mut length_bytes = [0; 4];
    stream.read_exact(&mut length_bytes).await?;

    let length = u32::from_be_bytes(length_bytes) as usize;
    let mut buffer = vec![0; length];
    #[cfg(feature = "async-tcp")]
    stream.read_exact(&mut buffer).await?;

    let mut buffer: &[u8] = &buffer;
    let magic_header_client = take_str(&mut buffer)?;
    if magic_header_client != magic_header_server {
        return Err(Error::ApiMisMatch(format!(
            "failed to match magic header, expected: {}, got: {}",
            magic_header_server, magic_header_client
        )));
    }
    let fn_name = take_str(&mut buffer)?;
    Ok((fn_name, buffer.to_vec()))
}

#[cfg(not(feature = "async-tcp"))]
fn receive_data(stream: &mut TcpStream, magic_header_server: &str) -> Result<(String, Vec<u8>)> {
    let mut length_bytes = [0; 4];
    stream.read_exact(&mut length_bytes)?;

    let length = u32::from_be_bytes(length_bytes) as usize;
    let mut buffer = vec![0; length];
    stream.read_exact(&mut buffer)?;

    let mut buffer: &[u8] = &buffer;
    let magic_header_client = take_str(&mut buffer)?;
    if magic_header_client != magic_header_server {
        return Err(Error::ApiMisMatch(format!(
            "failed to match magic header, expected: {}, got: {}",
            magic_header_server, magic_header_client
        )));
    }
    let fn_name = take_str(&mut buffer)?;
    Ok((fn_name, buffer.to_vec()))
}

type AsyncFuture<T> =
    fn(String, Vec<u8>, Arc<Mutex<T>>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;
async fn handle_client<T>(
    mut stream: TcpStream,
    magic_header: &str,
    future: AsyncFuture<T>,
    app_data: Arc<Mutex<T>>,
) {
    #[cfg(feature = "async-tcp")]
    let data = receive_data(&mut stream, magic_header).await;
    #[cfg(not(feature = "async-tcp"))]
    let data = receive_data(&mut stream, magic_header);
    let (func, data) = match data {
        Ok(v) => v,
        Err(err) => {
            let mut response_buffer = Vec::new();
            response_buffer.extend_from_slice(&[0, 0, 0, 1]);
            if let Ok(err) = encode(&err) {
                response_buffer.extend_from_slice(&err);
            }
            #[cfg(feature = "async-tcp")]
            let _ = stream.write_all(&response_buffer).await;
            #[cfg(not(feature = "async-tcp"))]
            let _ = stream.write_all(&response_buffer);
            return;
        }
    };

    let response = future(func, data, app_data).await;

    let mut response_buffer = Vec::new();
    match response {
        Ok(data) => {
            response_buffer.extend_from_slice(&[0, 0, 0, 0]);
            response_buffer.extend_from_slice(&data);
        }
        Err(err) => {
            response_buffer.extend_from_slice(&[0, 0, 0, 1]);
            if let Ok(err) = encode(&err) {
                response_buffer.extend_from_slice(&err);
            }
        }
    }
    #[cfg(feature = "async-tcp")]
    let _ = stream.write_all(&response_buffer).await;
    #[cfg(not(feature = "async-tcp"))]
    let _ = stream.write_all(&response_buffer);
}

pub trait Receiver<T: Send + 'static> {
    fn request(
        func: String,
        data: Vec<u8>,
        app_data: Arc<Mutex<T>>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;

    fn get_app_data(&self) -> Arc<Mutex<T>>;

    #[allow(async_fn_in_trait)]
    async fn start_from_listener(
        &self,
        listener: TcpListener,
        magic_header: &str,
    ) -> io::Result<()> {
        let thread_count = 8;

        let futures = Arc::new(Mutex::new(vec![]));
        let notify = Arc::new(Notify::new());
        for _ in 0..thread_count {
            let futures = futures.clone();
            let app_data = self.get_app_data();
            let notify = notify.clone();
            let magic_header = magic_header.to_owned();
            tokio::spawn(async move {
                loop {
                    notify.notified().await;
                    let item = futures.lock().await.pop();
                    if let Some(stream) = item {
                        handle_client(stream, &magic_header, Self::request, app_data.clone()).await;
                    }
                }
            });
        }

        loop {
            #[cfg(feature = "async-tcp")]
            if let Ok((stream, _)) = listener.accept().await {
                futures.lock().await.push(stream);
                notify.notify_one();
            }
            #[cfg(not(feature = "async-tcp"))]
            if let Ok((stream, _)) = listener.accept() {
                futures.lock().await.push(stream);
                notify.notify_one();
            }
        }
    }

    #[allow(async_fn_in_trait)]
    async fn start(&self, port: u16, magic_header: &str) -> io::Result<()> {
        let listener = create_listener(port).await?;
        self.start_from_listener(listener, magic_header).await
    }
}

async fn create_listener(port: u16) -> io::Result<TcpListener> {
    #[cfg(feature = "async-tcp")]
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    #[cfg(not(feature = "async-tcp"))]
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
    Ok(listener)
}

pub trait Starter {
    #[allow(async_fn_in_trait)]
    async fn start(self, port: u16, header: &str) -> std::io::Result<()>;
    #[allow(async_fn_in_trait)]
    async fn start_from_listener(self, listener: TcpListener, header: &str) -> std::io::Result<()>;
    #[allow(async_fn_in_trait)]
    async fn start_gen<T: Starter>(
        port: u16,
        magic_header: &str,
        gen: impl FnOnce() -> T,
    ) -> std::io::Result<()> {
        let listener = create_listener(port).await.unwrap();
        let app_data = gen();
        app_data.start_from_listener(listener, magic_header).await
    }
}
