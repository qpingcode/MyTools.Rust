use std::{net::Ipv4Addr, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::Html,
};
use mytools_plugin_api::i18n;
use qrcode::{QrCode, render::svg};
use serde::Serialize;

use crate::{
    CHINESE, ENGLISH,
    server::{Shared, lan_addresses},
};

pub async fn page() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

pub async fn style() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../web/style.css"),
    )
}

pub async fn script() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../web/app.js"),
    )
}

#[derive(Serialize)]
pub struct Connection {
    language: &'static str,
    english: &'static str,
    localized: &'static str,
    links: Vec<Link>,
}

#[derive(Serialize)]
struct Link {
    ip: Ipv4Addr,
    url: String,
}

fn url(state: &Shared, ip: Ipv4Addr) -> String {
    format!("http://{ip}:{}{}/", state.port, state.prefix)
}

pub async fn session(State(state): State<Arc<Shared>>) -> Json<Connection> {
    let language = i18n::language();
    Json(Connection {
        language,
        english: ENGLISH,
        localized: if language == "zh-CN" {
            CHINESE
        } else {
            ENGLISH
        },
        links: lan_addresses()
            .into_iter()
            .map(|ip| Link {
                ip,
                url: url(&state, ip),
            })
            .collect(),
    })
}

pub async fn qr(
    State(state): State<Arc<Shared>>,
    Path(ip): Path<Ipv4Addr>,
) -> Result<([(header::HeaderName, &'static str); 1], String), StatusCode> {
    if !lan_addresses().contains(&ip) {
        return Err(StatusCode::NOT_FOUND);
    }
    let code = QrCode::new(url(&state, ip)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let image = code.render::<svg::Color>().min_dimensions(240, 240).build();
    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], image))
}
