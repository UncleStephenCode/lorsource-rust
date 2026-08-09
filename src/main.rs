mod application;
mod audit;
mod auth;
mod bootstrap;
mod config;
mod csrf;
mod db;
mod domain;
mod error;
mod exception_report;
mod form;
mod image_upload;
mod infra;
mod markup;
mod models;
mod pagination;
mod profile;
mod request_timezone;
mod routes;
mod search_index;
mod security;
mod security_headers;
mod state;
mod theme_middleware;

use anyhow::Context;
use axum::{Router, routing::get};
use config::Config;
use state::AppState;
use std::net::SocketAddr;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lorsource_rust=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env().context("failed to load runtime configuration")?;
    let sEnvironment = std::env::var("LOR_ENV").unwrap_or_else(|_| "development".to_owned());
    config
        .vValidateForEnvironment(&sEnvironment)
        .context("invalid runtime configuration")?;
    let pool = db::connect(&config.database_url).await?;
    db::verify_schema(&pool).await?;

    std::fs::create_dir_all(format!("{}/photos", config.upload_dir))
        .context("failed to create upload photos directory")?;
    std::fs::create_dir_all(format!("{}/gallery", config.upload_dir))
        .context("failed to create upload gallery directory")?;
    std::fs::create_dir_all(format!("{}/gallery/preview", config.upload_dir))
        .context("failed to create gallery preview directory")?;
    std::fs::create_dir_all(format!("{}/images", config.upload_dir))
        .context("failed to create upload images directory")?;
    for sQueueDirectory in ["pending", "processing", "failed"] {
        std::fs::create_dir_all(format!(
            "{}/search-queue/{sQueueDirectory}",
            config.upload_dir
        ))
        .context("failed to create durable search queue directory")?;
    }

    let state = AppState::new(config.clone(), pool);
    if let Err(sError) = search_index::ensure_index(&state).await {
        if matches!(
            sEnvironment.trim().to_ascii_lowercase().as_str(),
            "production" | "prod"
        ) {
            anyhow::bail!(sError);
        }
        tracing::warn!(error = %sError, "failed to validate the OpenSearch index");
    }
    let (oShutdownSender, oShutdownReceiver) = tokio::sync::watch::channel(false);
    let vecBackgroundJobs = bootstrap::background::vecSpawn(state.clone(), oShutdownReceiver);
    let app = Router::new()
        .merge(routes::router())
        .route("/healthz", get(routes::healthz))
        .route("/readyz", get(routes::readyz))
        .route_service(
            "/favicon.ico",
            ServeFile::new(format!("{}/favicon.ico", config.static_dir)),
        )
        .route_service(
            "/manifest.json",
            ServeFile::new(format!("{}/manifest.json", config.static_dir)),
        )
        .route_service(
            "/robots.txt",
            ServeFile::new(format!("{}/robots.txt", config.static_dir)),
        )
        .route_service(
            "/googlea3fb422736ed276d.html",
            ServeFile::new(format!("{}/googlea3fb422736ed276d.html", config.static_dir)),
        )
        .nest_service("/static", ServeDir::new(&config.static_dir))
        .nest_service("/img", ServeDir::new(format!("{}/img", config.static_dir)))
        .nest_service(
            "/font",
            ServeDir::new(format!("{}/font", config.static_dir)),
        )
        .nest_service("/js", ServeDir::new(format!("{}/js", config.static_dir)))
        .nest_service(
            "/webjars",
            ServeDir::new(format!("{}/webjars", config.static_dir)),
        )
        .nest_service(
            "/black",
            ServeDir::new(format!("{}/black", config.static_dir)),
        )
        .nest_service(
            "/tango",
            ServeDir::new(format!("{}/tango", config.static_dir)),
        )
        .nest_service(
            "/white2",
            ServeDir::new(format!("{}/white2", config.static_dir)),
        )
        .nest_service(
            "/waltz",
            ServeDir::new(format!("{}/waltz", config.static_dir)),
        )
        .nest_service(
            "/zomg_ponies",
            ServeDir::new(format!("{}/zomg_ponies", config.static_dir)),
        )
        .nest_service("/adv", ServeDir::new(format!("{}/adv", config.static_dir)))
        .nest_service(
            "/qrerror",
            ServeDir::new(format!("{}/qrerror", config.static_dir)),
        )
        .fallback(routes::not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            routes::adv::apply,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::hydrate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            exception_report::apply,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security_headers::apply,
        ))
        .layer(axum::middleware::from_fn(routes::static_cache::apply))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            theme_middleware::apply_theme,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf::apply,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            routes::canonical_host::apply,
        ))
        // This is the outermost application middleware, matching web.xml where
        // UrlRewriteFilter runs before Spring Security and DispatcherServlet.
        .layer(axum::middleware::from_fn(routes::legacy_redirects::apply))
        .with_state(state.clone());

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context("invalid LISTEN address")?;

    tracing::info!(%addr, "starting lorsource-rust");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Java's AddTopicChecker receives `request.getRemoteAddr` through
    // AnySession/IpBlockInfo.  Preserve the TCP peer address in request
    // extensions so `/add.jsp` can query the canonical `b_ips` table.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(vShutdownSignal(oShutdownSender))
    .await?;
    for mut stJob in vecBackgroundJobs {
        if tokio::time::timeout(std::time::Duration::from_secs(15), &mut stJob)
            .await
            .is_err()
        {
            stJob.abort();
        }
    }
    state.pool.close().await;
    Ok(())
}

async fn vShutdownSignal(oShutdown: tokio::sync::watch::Sender<bool>) {
    let oCtrlC = async {
        if let Err(stError) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %stError, "failed to install Ctrl-C signal handler");
        }
    };

    #[cfg(unix)]
    let oTerminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut oSignal) => {
                oSignal.recv().await;
            }
            Err(stError) => {
                tracing::error!(error = %stError, "failed to install SIGTERM signal handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let oTerminate = std::future::pending::<()>();

    tokio::select! {
        () = oCtrlC => {}
        () = oTerminate => {}
    }
    let _ = oShutdown.send(true);
    tracing::info!("shutdown signal received; draining HTTP connections");
}
