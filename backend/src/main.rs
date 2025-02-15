use axum::{
    http::StatusCode,
    response::{
        IntoResponse,
        Response
    },
    routing::{get, post}, 
    Router
};

use tracing_subscriber;

use anyhow::Result;
use controllers::{
    logs::{
        get_log, get_logs_count, list_logs
    }, models::list_models, prompt_samples::{create_sample, delete_sample, get_sample_by_id, get_samples_by_prompt, update_sample}, prompts::{
        create_prompt, delete_prompt, execute_prompt, execute_prompt_stream, get_prompt, list_prompts, update_prompt
    }
};

use db::{init::DbData, types::prompt::PromptRowWithModel};
use moka::future::Cache;

pub mod common;
pub mod db;
pub mod controllers;
pub mod services;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")?;
    let log_level = std::env::var("RUST_LOG").unwrap_or("info".to_string());

    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    let data = DbData::new(&database_url).await?;
    let app_state = AppState::new(data).await;

    let router = Router::new()
        .nest("/api/v1", api_v1_routes())
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;

    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, router).await.unwrap();

    Ok(())
}

fn api_v1_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(api_version_handler))
        .nest("/prompts", prompt_routes())
        .nest("/prompt-samples", prompt_samples_routes())
        .nest("/prompts/execute", execute_routes())
        .nest("/models", model_routes())
        .nest("/logs", logs_routes())
}

fn execute_routes() -> Router<AppState> {
    Router::new()
        .route("/{id}", post(execute_prompt))
        .route("/{id}/stream", post(execute_prompt_stream))
}

fn prompt_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_prompt).get(list_prompts))
        .route("/{id}", get(get_prompt).put(update_prompt).delete(delete_prompt))
        .route("/{id}/prompt-samples", get(get_samples_by_prompt))
}

fn prompt_samples_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_sample))
        .route("/{id}", get(get_sample_by_id).put(update_sample).delete(delete_sample))
}

fn model_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_models))
}

fn logs_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_logs))
        .route("/{trace_id}", get(get_log))
        .route("/count", get(get_logs_count))
}

async fn api_version_handler() -> &'static str {
    "llmkit api v0.1"
}

// APP STATE
#[derive(Clone)]
pub struct AppState {
    pub db: DbData,
    pub prompt_cache: Cache<i64, PromptRowWithModel>,
}

impl AppState {
    pub async fn new(data: DbData) -> Self {
        let prompt_cache: Cache<i64, PromptRowWithModel> = Cache::new(500);

        AppState { db: data, prompt_cache }
    }
}


// ANYHOW ERROR HANDLING
#[allow(dead_code)]
#[derive(Debug)]
pub enum AppError {
    Unauthorized(String),
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    InternalServerError(String),
    TooManyRequests(String),
    Other(anyhow::Error)
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Unauthorized(e) => {
                tracing::error!("Unauthorized | error: {}", e);
                return (StatusCode::UNAUTHORIZED, format!("Unauthorized: {:?}", e)).into_response();
            },
            AppError::BadRequest(e) => {
                tracing::error!("Bad Request | error: {}", e);
                return (StatusCode::BAD_REQUEST, format!("Bad Request: {:?}", e)).into_response();
            }
            AppError::NotFound(e) => {
                tracing::error!("Not Found | error: {}", e);
                return (StatusCode::NOT_FOUND, format!("{}", e)).into_response();
            }
            AppError::Conflict(e) => {
                tracing::error!("Conflict | error: {}", e);
                return (StatusCode::CONFLICT, format!("{}", e)).into_response();
            }
            AppError::InternalServerError(e) => {
                tracing::error!("Internal server error | error: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)).into_response();
            }
            AppError::TooManyRequests(e) => {
                tracing::error!("Too many requests | error: {}", e);
                return (StatusCode::TOO_MANY_REQUESTS, format!("{}", e)).into_response();
            }
            AppError::Other(e) => {
                tracing::error!("Internal Server Error | error: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("Internal Server Error")).into_response();
            }

        }
        
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Other(err)
    }
}

