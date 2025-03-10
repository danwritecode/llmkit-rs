use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc::Sender;
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};
use tracing;

use super::{
    providers::openrouter::OpenrouterProvider,
    types::{
        chat_request::LlmServiceRequest,
        chat_response::{LlmServiceChatCompletionChunk, LlmServiceChatCompletionResponse},
        llm_error::{LlmError, LlmStreamingError},
    },
};
use crate::{common::types::models::LlmApiProvider, db::logs::LogRepository};

pub struct Llm {
    props: LlmServiceRequest,
    db_log: LogRepository,
}

impl Llm {
    pub fn new(props: LlmServiceRequest, db_log: LogRepository) -> Self {
        Llm { props, db_log }
    }

    fn retry_strategy(&self) -> impl Iterator<Item = Duration> {
        ExponentialBackoff::from_millis(100)
            .max_delay(Duration::from_secs(100))
            .map(jitter)
            .take(3)
    }

    pub async fn text(&self) -> Result<(LlmServiceChatCompletionResponse, i64), LlmError> {
        let retry_strategy = self.retry_strategy();
        Retry::spawn(retry_strategy, || self.send_request()).await
    }

    pub async fn json(&self) -> Result<(LlmServiceChatCompletionResponse, i64), LlmError> {
        let retry_strategy = self.retry_strategy();
        Retry::spawn(retry_strategy, || async {
            let res = self.send_request().await?;
            // Since this is not a client library and will be invoked via API
            // we can't strongly enforce the shape of the JSON, therefore we just
            // need to make sure it is a valid JSON (hence Value) and then convert
            // it back into text and be on our way
            if let Some(c) = res.0.choices.first() {
                let _json: serde_json::Value = serde_json::from_str(&c.message.content)?;
            }

            Ok(res)
        })
        .await
    }

    pub async fn stream(
        &self,
        tx: Sender<Result<LlmServiceChatCompletionChunk, LlmStreamingError>>,
    ) -> Result<(LlmServiceChatCompletionResponse, i64), LlmError> {
        if self.props.request.response_format.is_some() {
            tracing::info!("Json mode not supported in chat mode");
            return Err(LlmError::UnsupportedMode(
                "Json".to_string(),
                "Chat".to_string(),
            ));
        }

        Ok(self.send_request_stream(tx).await?)
    }

    async fn send_request(&self) -> Result<(LlmServiceChatCompletionResponse, i64), LlmError> {
        // Initialize variables to capture data even in error cases
        let mut input_tokens = None;
        let mut output_tokens = None;
        let reasoning_tokens = None;
        let mut raw_response: Option<String> = None;
        let mut status = Some(500); // Default to error status

        // Serialize the request for logging
        let request_body = serde_json::to_string(&self.props)
            .map_err(|e| LlmError::SerializationError(e.to_string()))?;

        // Execute request and capture result
        let result = match &self.props.provider {
            LlmApiProvider::Openrouter => {
                let provider = OpenrouterProvider::new(&self.props, false)?;
                provider.execute_chat().await
            }
        };

        // Process the result or prepare error
        let (exec_result, provider_response_id) = match result {
            Ok(provider_response) => {
                // Update status for successful request
                status = Some(200);

                // Extract tokens and usage information
                input_tokens = provider_response
                    .usage
                    .as_ref()
                    .map(|usage| usage.prompt_tokens as i64);

                output_tokens = provider_response
                    .usage
                    .as_ref()
                    .map(|usage| usage.completion_tokens as i64);

                // Save raw response for logging
                raw_response = serde_json::to_string(&provider_response).ok();

                // Extract content from the response
                if provider_response.choices.len() > 0 {
                    // Save response ID for logging
                    let id = provider_response.id.clone();
                    (Ok(provider_response), id)
                } else {
                    (Err(LlmError::EmptyResponse), uuid::Uuid::new_v4().to_string())
                }
            }
            Err(e) => {
                // For errors, prepare as much information as possible for logging
                raw_response = Some(format!("{{\"error\": \"{}\"}}", e));
                (Err(e), uuid::Uuid::new_v4().to_string())
            }
        };

        // Always log the request, regardless of success or failure
        let log_id = self
            .log_request(
                raw_response.as_deref(),
                status,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                &request_body,
                &provider_response_id,
            )
            .await?;

        // Return the original result but with the correct log_id
        match exec_result {
            Ok(r) => Ok((r, log_id)),
            Err(e) => Err(e),
        }
    }

    async fn send_request_stream(
        &self,
        tx: Sender<Result<LlmServiceChatCompletionChunk, LlmStreamingError>>,
    ) -> Result<(LlmServiceChatCompletionResponse, i64), LlmError> {
        // Initialize variables to capture data even in error cases
        let mut input_tokens = None;
        let mut output_tokens = None;
        let reasoning_tokens = None;
        let mut raw_response: Option<String> = None;
        let mut status = Some(500); // Default to error status

        // Serialize the request for logging
        let request_body = serde_json::to_string(&self.props)
            .map_err(|e| LlmError::SerializationError(e.to_string()))?;

        // Check json mode before making the request
        if self.props.request.response_format.is_some() {
            tracing::info!("Json mode not supported in chat mode");
            let error = LlmError::UnsupportedMode("Json".to_string(), "Chat".to_string());

            // Log the error
            raw_response = Some(format!("{{\"error\": \"{}\"}}", error));
            let provider_response_id = uuid::Uuid::new_v4().to_string();

            // Log the failed request
            self.log_request(
                raw_response.as_deref(),
                status,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                &request_body,
                &provider_response_id
            )
            .await?;

            return Err(error);
        }

        // Execute request and capture result
        let result = match &self.props.provider {
            LlmApiProvider::Openrouter => {
                let provider = OpenrouterProvider::new(&self.props, true)?;
                provider.execute_chat_stream(tx).await
            }
        };

        // Process the result or prepare error
        let (exec_result, provider_response_id) = match result {
            Ok(response) => {
                // Update status for successful request
                status = Some(200);

                // Extract tokens and usage information
                input_tokens = response
                    .usage
                    .as_ref()
                    .map(|usage| usage.prompt_tokens as i64);

                output_tokens = response
                    .usage
                    .as_ref()
                    .map(|usage| usage.completion_tokens as i64);

                // Save raw response for logging
                raw_response = serde_json::to_string(&response).ok();

                // Extract content from the response
                if response.choices.len() > 0 {
                    let id = response.id.clone();
                    (Ok(response), id)
                } else {
                    (Err(LlmError::EmptyResponse), uuid::Uuid::new_v4().to_string())
                }
            }
            Err(e) => {
                // For errors, prepare as much information as possible for logging
                raw_response = Some(format!("{{\"error\": \"{}\"}}", e));
                (Err(e), uuid::Uuid::new_v4().to_string())
            }
        };

        // Always log the request, regardless of success or failure
        let log_id = self
            .log_request(
                raw_response.as_deref(),
                status,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                &request_body,
                &provider_response_id
            )
            .await?;

        // Return the original result but with the correct log_id
        match exec_result {
            Ok(r) => Ok((r, log_id)),
            Err(e) => Err(e),
        }
    }

    /// Logs the request and returns a log ID.
    async fn log_request(
        &self,
        raw_response: Option<&str>,
        status: Option<i64>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        reasoning_tokens: Option<i64>,
        request_body: &str,
        provider_response_id: &str,
    ) -> Result<i64, LlmError> {
        self.db_log
            .create_log(
                Some(self.props.prompt_id),
                self.props.model_id,
                raw_response,
                status,
                input_tokens,
                output_tokens,
                reasoning_tokens,
                Some(request_body),
                provider_response_id,
            )
            .await
            .map_err(|e| LlmError::DbLoggingError(e.to_string()))
    }
}
