use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::Serialize;
use tokio::{runtime::Runtime, sync::Semaphore, time::timeout};
use uuid::Uuid;

use crate::web;

const FILE_LIMIT: usize = 64 * 1024 * 1024;
const TEXT_LIMIT: usize = 64 * 1024;
const SESSION_LIMIT: usize = 256 * 1024 * 1024;
const ITEM_LIMIT: usize = 256;

pub struct Session {
    // Dropping the runtime closes the listener and cancels active transfers.
    _runtime: Runtime,
    address: String,
}

impl Session {
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let prefix = format!("/{}", Uuid::new_v4().simple());
        let address = format!("http://127.0.0.1:{port}{prefix}/");
        let state = Arc::new(Shared {
            port,
            prefix,
            shelf: Mutex::new(Shelf::default()),
            uploads: Semaphore::new(4),
        });
        let app = routes(state);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let listener = {
            let _entered = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        runtime.spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                mytools_plugin_api::log::error("lan_transfer.server", &error.to_string());
            }
        });
        Ok(Self {
            _runtime: runtime,
            address,
        })
    }

    pub fn url(&self) -> String {
        self.address.clone()
    }
}

pub struct Shared {
    pub port: u16,
    pub prefix: String,
    shelf: Mutex<Shelf>,
    uploads: Semaphore,
}

#[derive(Default)]
struct Shelf {
    items: Vec<Entry>,
    size: usize,
}

struct Entry {
    info: Item,
    data: Bytes,
}

#[derive(Clone, Serialize)]
struct Item {
    id: Uuid,
    filename: String,
    size: usize,
    text_preview: Option<String>,
}

fn routes(state: Arc<Shared>) -> Router {
    let path = |suffix: &str| format!("{}{suffix}", state.prefix);
    Router::new()
        .route(&path("/"), get(web::page))
        .route(&path("/style.css"), get(web::style))
        .route(&path("/app.js"), get(web::script))
        .route(&path("/session"), get(web::session))
        .route(&path("/qr/{ip}"), get(web::qr))
        .route(&path("/items"), get(list).post(upload))
        .route(&path("/items/{id}"), get(download).delete(delete))
        .layer(middleware::from_fn(response_headers))
        .with_state(state)
}

async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
    ));
    response
}

async fn list(State(state): State<Arc<Shared>>) -> Json<Vec<Item>> {
    let shelf = state.shelf.lock().unwrap();
    Json(
        shelf
            .items
            .iter()
            .rev()
            .map(|item| item.info.clone())
            .collect(),
    )
}

async fn upload(
    State(state): State<Arc<Shared>>,
    request: Request,
) -> Result<StatusCode, StatusCode> {
    let _slot = state
        .uploads
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let (headers, body) = request.into_parts();
    let filename = match headers.headers.get("x-transfer-filename") {
        Some(value) => {
            let encoded = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
            let decoded = percent_decode_str(encoded)
                .decode_utf8()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let name = decoded
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .trim();
            if name.is_empty()
                || matches!(name, "." | "..")
                || name.chars().any(char::is_control)
                || name.len() > 1024
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            Some(name.to_owned())
        }
        None if headers
            .headers
            .get("x-transfer-text")
            .is_some_and(|value| value == "1") =>
        {
            None
        }
        None => return Err(StatusCode::BAD_REQUEST),
    };
    let limit = if filename.is_some() {
        FILE_LIMIT
    } else {
        TEXT_LIMIT
    };
    if headers
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<usize>().ok())
        .is_some_and(|len| len > limit)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let data = timeout(Duration::from_secs(180), to_bytes(body, limit))
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|error| {
            if std::error::Error::source(&error)
                .is_some_and(|cause| cause.is::<http_body_util::LengthLimitError>())
            {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            }
        })?;
    let text_preview = if filename.is_none() {
        let text = std::str::from_utf8(&data).map_err(|_| StatusCode::BAD_REQUEST)?;
        if text.trim().is_empty() {
            return Err(StatusCode::BAD_REQUEST);
        }
        Some(text.chars().take(240).collect())
    } else {
        None
    };
    let mut shelf = state.shelf.lock().unwrap();
    if shelf.items.len() >= ITEM_LIMIT || shelf.size + data.len() > SESSION_LIMIT {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    let info = Item {
        id: Uuid::new_v4(),
        filename: filename.unwrap_or_else(|| "message.txt".into()),
        size: data.len(),
        text_preview,
    };
    shelf.size += data.len();
    shelf.items.push(Entry { info, data });
    Ok(StatusCode::CREATED)
}

async fn download(
    State(state): State<Arc<Shared>>,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let shelf = state.shelf.lock().unwrap();
    let entry = shelf
        .items
        .iter()
        .find(|entry| entry.info.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let filename = utf8_percent_encode(&entry.info.filename, NON_ALPHANUMERIC);
    let disposition = format!("attachment; filename*=UTF-8''{filename}");
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
        ],
        entry.data.clone(),
    )
        .into_response())
}

async fn delete(
    State(state): State<Arc<Shared>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let mut shelf = state.shelf.lock().unwrap();
    let index = shelf
        .items
        .iter()
        .position(|entry| entry.info.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let entry = shelf.items.remove(index);
    shelf.size -= entry.data.len();
    Ok(StatusCode::NO_CONTENT)
}

pub fn lan_addresses() -> Vec<Ipv4Addr> {
    let mut addresses: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            std::net::IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() => Some(ip),
            _ => None,
        })
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    // Prefer the interface the OS uses for its default route. UDP connect sends no packet.
    let preferred = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
        socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
        socket.local_addr()
    });
    if let Ok(SocketAddr::V4(preferred)) = preferred {
        addresses.sort_by_key(|ip| ip != preferred.ip());
    }
    addresses
}
