use std::{env, path::PathBuf};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Router,
};
use axum::middleware as axum_middleware;

use middleware::auth::{self, user_auth_middleware};
use services::types::llm_error::LlmError;
use tower_cookies::CookieManagerLayer;
use tracing_subscriber;

use anyhow::Result;
use controllers::{
    api_keys::{create_api_key, delete_api_key, list_api_keys},
    logs::{get_log, get_log_by_provider_id, get_logs_count, list_logs},
    models::{list_models, create_model, update_model},
    providers::list_providers,
    prompt_eval::{
        create_eval_test, delete_eval_test, get_eval_test_by_id, get_eval_test_by_prompt,
        update_eval_test,
    },
    prompt_eval_run::{
        execute_eval_run, get_eval_performance_by_prompt_id, get_eval_run_by_id,
        get_eval_runs_by_prompt_version, update_eval_run_score,
    },
    prompts::{
        api_completions, api_completions_stream, create_prompt, delete_prompt, get_prompt, 
        get_prompt_versions, list_prompts, set_active_version, update_prompt
    }, 
    schema::validate_schema,
    tools::{
        associate_tool_with_prompt_version, create_tool, delete_tool, 
        get_prompt_versions_by_tool, get_tool, get_tools_by_prompt_version, 
        list_tools, remove_tool_prompt_version_association, update_tool
    },
    user::{login, register, me},
};

use db::{init::DbData, types::prompt::PromptRowWithModel};
use moka::future::Cache;

pub mod common;
pub mod controllers;
pub mod db;
pub mod services;
pub mod middleware;
pub mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    path.pop();
    path.push(".env");

    dotenv::from_path(path.as_path())?;

    let database_url = env::var("DATABASE_URL")?;
    let log_level = env::var("RUST_LOG").unwrap_or("info".to_string());

    tracing_subscriber::fmt().with_env_filter(log_level).init();

    let data = DbData::new(&database_url).await?;
    let app_state = AppState::new(data).await;

    // API routes that require API key auth
    let api_routes = Router::new()
        .route("/chat/completions", post(api_completions))
        .route("/chat/completions/stream", post(api_completions_stream))
        .layer(axum_middleware::from_fn_with_state(
            app_state.clone(),
            auth::api_key_middleware,
        ));

    // Public routes (no auth required)
    let public_routes = Router::new()
        .route("/", get(api_version_handler))
        .route("/ui/auth/register", post(register))
        .route("/ui/auth/login", post(login))
        .layer(CookieManagerLayer::new());

    // User authenticated routes
    let user_routes = Router::new()
        .route("/ui/auth/me", get(me))
        .route("/ui/settings/api-keys", get(list_api_keys).post(create_api_key))
        .route("/ui/settings/api-keys/{id}", delete(delete_api_key))
        .route("/ui/prompts", post(create_prompt).get(list_prompts))
        .route("/ui/prompts/{id}", get(get_prompt).put(update_prompt).delete(delete_prompt))
        .route("/ui/prompts/{id}/versions", get(get_prompt_versions))
        .route("/ui/prompts/{prompt_id}/set-version/{version_id}", put(set_active_version))
        .route("/ui/prompts/{id}/prompt-evals", get(get_eval_test_by_prompt))
        .route("/ui/prompts/{id}/performance", get(get_eval_performance_by_prompt_id))
        .route("/ui/prompts/execute", post(api_completions))
        .route("/ui/prompts/execute/stream", post(api_completions_stream))
        .route("/ui/prompts/execute/chat", post(api_completions))
        .route("/ui/prompts/execute/chat/stream", post(api_completions_stream))
        .route("/ui/prompt-evals", post(create_eval_test))
        .route("/ui/prompt-evals/{id}", get(get_eval_test_by_id).put(update_eval_test).delete(delete_eval_test))
        .route("/ui/prompt-eval-runs/{prompt_id}/version/{prompt_version_id}", post(execute_eval_run).get(get_eval_runs_by_prompt_version))
        .route("/ui/prompt-eval-runs/{id}",get(get_eval_run_by_id).put(update_eval_run_score))
        .route("/ui/models", get(list_models).post(create_model))
        .route("/ui/models/{id}", put(update_model))
        .route("/ui/providers", get(list_providers))
        .route("/ui/logs", get(list_logs))
        .route("/ui/logs/count", get(get_logs_count))
        .route("/ui/logs/provider/{provider_id}", get(get_log_by_provider_id))
        .route("/ui/logs/{trace_id}", get(get_log))
        .route("/ui/schema/validate", post(validate_schema))
        .route("/ui/tools", post(create_tool).get(list_tools))
        .route("/ui/tools/{id}", get(get_tool).put(update_tool).delete(delete_tool))
        .route("/ui/tools/associate", post(associate_tool_with_prompt_version))
        .route("/ui/tools/disassociate", post(remove_tool_prompt_version_association))
        .route("/ui/tools/{id}/prompts", get(get_prompt_versions_by_tool))
        .route("/ui/prompts/versions/{id}/tools", get(get_tools_by_prompt_version))
        .layer(axum_middleware::from_fn_with_state(app_state.clone(), user_auth_middleware))
        .layer(CookieManagerLayer::new());

    // Combine all routes into the main router
    let router = Router::new()
        .nest(
            "/v1",
            Router::new()
                .merge(api_routes)
                .merge(public_routes)
                // .merge(admin_routes)
                .merge(user_routes),
        )
        .with_state(app_state);

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or("0.0.0.0:8000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_address).await?;

    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, router).await.unwrap();

    Ok(())
}

async fn api_version_handler() -> &'static str {
    "llmkit api v0.1"
}

// APP STATE
#[derive(Clone)]
pub struct AppState {
    pub db: DbData,
    pub prompt_cache: Cache<i64, PromptRowWithModel>,
    pub jwt_secret: String
}

impl AppState {
    pub async fn new(data: DbData) -> Self {
        let prompt_cache: Cache<i64, PromptRowWithModel> = Cache::new(500);
        let jwt_secret = std::env::var("JWT_SECRET").expect("Missing JWT_SECRET from env vars");

        AppState {
            db: data,
            prompt_cache,
            jwt_secret
        }
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
    Forbidden(String),
    Other(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Unauthorized(e) => {
                tracing::error!("Unauthorized | error: {}", e);
                return (StatusCode::UNAUTHORIZED, format!("Unauthorized: {:?}", e))
                    .into_response();
            }
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
            AppError::Forbidden(e) => {
                tracing::error!("Forbidden | error: {}", e);
                return (StatusCode::FORBIDDEN, format!("{}", e)).into_response();
            }
            AppError::Other(e) => {
                tracing::error!("Internal Server Error | error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Internal Server Error"),
                )
                    .into_response();
            }
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Other(err)
    }
}

impl From<LlmError> for AppError {
    fn from(err: LlmError) -> Self {
        match err {
            // Auth errors
            LlmError::Auth(msg) => AppError::Unauthorized(msg),
            LlmError::InvalidApiKey => AppError::Unauthorized("API key invalid or expired".to_string()),
            LlmError::InsufficientPermissions => AppError::Forbidden("Insufficient permissions".to_string()),
            
            // Rate limits
            LlmError::RateLimit(msg) => AppError::TooManyRequests(msg),
            LlmError::ProviderQuotaExceeded => AppError::TooManyRequests("Provider quota exceeded".to_string()),
            
            // Not found errors
            LlmError::ModelNotFound(msg) => AppError::NotFound(msg),
            LlmError::NotFound(msg) => AppError::NotFound(msg),
            
            // Bad request errors
            LlmError::MissingField(msg) => AppError::BadRequest(format!("Missing field: {}", msg)),
            LlmError::InvalidRole(msg) => AppError::BadRequest(format!("Invalid role: {}", msg)),
            LlmError::UnsupportedMode(mode, context) => AppError::BadRequest(format!("{} not supported in {}", mode, context)),
            LlmError::MissingSystemMessage => AppError::BadRequest("Missing system message".to_string()),
            LlmError::MissingUserMessage => AppError::BadRequest("Missing user message".to_string()),
            LlmError::PromptTooLong(current, limit) => AppError::BadRequest(format!("Prompt exceeds token limit: {}/{}", current, limit)),
            LlmError::ContentPolicy(msg) => AppError::BadRequest(format!("Content policy violation: {}", msg)),
            LlmError::InvalidConfig(msg) => AppError::BadRequest(format!("Invalid configuration: {}", msg)),
            
            // All network/http errors map to internal server error
            LlmError::Http(status) => {
                if status.as_u16() == 429 {
                    AppError::TooManyRequests(format!("HTTP status {}", status))
                } else if status.as_u16() == 404 {
                    AppError::NotFound(format!("Resource not found (HTTP {})", status))
                } else if status.as_u16() >= 400 && status.as_u16() < 500 {
                    AppError::BadRequest(format!("HTTP error: {}", status))
                } else {
                    AppError::InternalServerError(format!("HTTP error: {}", status))
                }
            },
            
            // All other errors map to internal server error
            _ => AppError::InternalServerError(format!("{}", err)),
        }
    }
}
