#[derive(Debug, Clone)]
pub struct StConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub public_url: String,
    pub static_dir: String,
    pub upload_dir: String,
    pub cookie_secret: String,
    pub site_secret: String,
    pub opensearch_url: Option<String>,
    pub smtp_url: Option<String>,
    pub run_migrations: bool,
    pub page_size: i64,
}

pub type Config = StConfig;

impl StConfig {
    pub fn stFromEnv() -> Self {
        Self {
            host: std::env::var("LOR_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("LOR_PORT").ok().and_then(|sValue| sValue.parse().ok()).unwrap_or(8080),
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://lor:lor@localhost:5432/lor".to_string()),
            public_url: std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8080".to_string()),
            static_dir: std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string()),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".to_string()),
            cookie_secret: std::env::var("COOKIE_SECRET").unwrap_or_else(|_| "dev-only-change-me-change-me-change-me".to_string()),
            site_secret: std::env::var("SITE_SECRET").ok().or_else(|| std::env::var("COOKIE_SECRET").ok()).unwrap_or_else(|| "dev-only-change-me-change-me-change-me".to_string()),
            opensearch_url: std::env::var("OPENSEARCH_URL").ok(),
            smtp_url: std::env::var("SMTP_URL").ok(),
            run_migrations: std::env::var("RUN_MIGRATIONS").map(|sValue| sValue != "0" && sValue != "false").unwrap_or(true),
            page_size: std::env::var("PAGE_SIZE").ok().and_then(|sValue| sValue.parse().ok()).unwrap_or(30),
        }
    }

    pub fn from_env() -> Self {
        Self::stFromEnv()
    }
}
