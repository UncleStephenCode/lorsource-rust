mod audit;
mod auth;
mod config;
mod db;
mod bootstrap;
mod application;
mod csrf;
mod infra;
mod domain;
mod error;
mod form;
mod markup;
mod models;
mod models_compat;
mod security;
mod pagination;
mod profile;
mod routes;
mod search_index;
mod security_headers;
mod state;
mod theme_middleware;

use anyhow::Context;
use axum::{routing::get, Router};
use config::Config;
use state::AppState;
use std::net::SocketAddr;
use tower_http::{compression::CompressionLayer, services::{ServeDir, ServeFile}, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "lorsource_rust=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    if config.run_migrations {
        db::migrate(&pool).await?;
    }

    std::fs::create_dir_all(format!("{}/photos", config.upload_dir))
        .context("failed to create upload photos directory")?;
    std::fs::create_dir_all(format!("{}/gallery", config.upload_dir))
        .context("failed to create upload gallery directory")?;
    std::fs::create_dir_all(format!("{}/images", config.upload_dir))
        .context("failed to create upload images directory")?;

    let state = AppState::new(config.clone(), pool);
    search_index::ensure_index(&state).await;
    let app = Router::new()
        .merge(routes::router())
        .route("/healthz", get(routes::healthz))
        .route_service("/favicon.ico", ServeFile::new(format!("{}/favicon.ico", &config.static_dir)))
        .nest_service("/static", ServeDir::new(&config.static_dir))
        .nest_service("/img", ServeDir::new(format!("{}/img", &config.static_dir)))
        .nest_service("/font", ServeDir::new(format!("{}/font", &config.static_dir)))
        .nest_service("/js", ServeDir::new(format!("{}/js", &config.static_dir)))
        .nest_service("/black", ServeDir::new(format!("{}/black", &config.static_dir)))
        .nest_service("/tango", ServeDir::new(format!("{}/tango", &config.static_dir)))
        .nest_service("/white2", ServeDir::new(format!("{}/white2", &config.static_dir)))
        .nest_service("/waltz", ServeDir::new(format!("{}/waltz", &config.static_dir)))
        .nest_service("/zomg_ponies", ServeDir::new(format!("{}/zomg_ponies", &config.static_dir)))
        .nest_service("/adv", ServeDir::new(format!("{}/adv", &config.static_dir)))
        .nest_service("/photos", ServeDir::new(format!("{}/photos", &config.upload_dir)))
        .nest_service("/gallery-uploads", ServeDir::new(format!("{}/gallery", &config.upload_dir)))
        .nest_service("/images", ServeDir::new(format!("{}/images", &config.upload_dir)))
        .fallback(routes::not_found)
        .layer(axum::middleware::from_fn_with_state(state.clone(), security_headers::apply))
        .layer(axum::middleware::from_fn_with_state(state.clone(), theme_middleware::apply_theme))
        .layer(axum::middleware::from_fn_with_state(state.clone(), csrf::apply))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context("invalid LISTEN address")?;

    tracing::info!(%addr, "starting lorsource-rust");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
