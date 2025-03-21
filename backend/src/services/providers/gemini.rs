use std::vec;

use crate::services::{
    llm::{Error, LlmProvider}, 
    types::{llm_props::LlmProps, message::Message, parse_response::LlmApiResponseProps, stream::LlmStreamingError}
};

use anyhow::Result;
use reqwest::RequestBuilder;
use reqwest_eventsource::Event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::Sender;
use futures_util::StreamExt;

pub struct GeminiProvider<'a> {
    props: &'a LlmProps,
    streaming: bool,
}

impl<'a> GeminiProvider<'a > {
    pub fn new(props: &'a LlmProps, streaming: bool) -> Self {
        GeminiProvider {
            props,
            streaming
        }
    }
}


#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GeminiResponse {
    candidates: Vec<GeminiResponseCandidate>,
    #[serde(rename = "promptFeedback")]
    prompt_feedback: Option<GeminiPromptFeedback>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseCandidate {
    content: GeminiMessageContent,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
    index: Option<i64>,
    #[serde(rename = "safetyRatings")]
    safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiMessageContent {
    parts: Vec<GeminiContentPart>,
    role: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiContentPart {
    text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiPromptFeedback {
    #[serde(rename = "blockReason")]
    block_reason: Option<String>,
    #[serde(rename = "safetyRatings")]
    safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: i64,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: i64,
    #[serde(rename = "totalTokenCount")]
    total_token_count: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiSafetyRating {
    category: String,
    probability: String,
}

// STREAMING
#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseStreamChunk {
    candidates: Vec<GeminiResponseStreamCandidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiResponseStreamUsage>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseStreamCandidate {
    content: GeminiResponseStreamContent,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseStreamContent {
    parts: Vec<GeminiResponseStreamPart>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseStreamPart {
    text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiResponseStreamUsage {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: i32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<i32>,
    #[serde(rename = "totalTokenCount")]
    total_token_count: i32,
}

impl From<GeminiResponse> for LlmApiResponseProps {
    fn from(response: GeminiResponse) -> Self {
        let response_content = response
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .and_then(|p| Some(p.text.clone()))
            .unwrap_or_default();

        // Extract token counts
        let (input_tokens, output_tokens) = response.clone().usage_metadata
            .map(|um| (Some(um.prompt_token_count), Some(um.candidates_token_count)))
            .unwrap_or((None, None));

        // Extract error information
        let (error_code, error_message) = response.clone().prompt_feedback
            .and_then(|pf| pf.block_reason
                .map(|br| (Some(br.clone()), Some(format!("Prompt blocked: {}", br)))))
            .unwrap_or((None, None));

        LlmApiResponseProps {
            response_content,
            raw_response: serde_json::to_string(&response).unwrap_or_default(),
            input_tokens,
            output_tokens,
            reasoning_tokens: None
        }
    }
}


impl<'a> LlmProvider for GeminiProvider<'a> {
    fn build_request(&self) -> Result<(RequestBuilder, String), Error> {
        let client = reqwest::Client::new();
        let api_key = std::env::var("GOOGLE_API_KEY").map_err(|_| Error::Auth)?;

        let body = self.create_body();
        let body_string = body.to_string();

        let url = format!(
            "{}/models/{}:{}",
            &self.props.base_url,
            self.props.model_name,
            if self.streaming { "streamGenerateContent" } else { "generateContent" }
        );

        let mut request = client.post(&url)
            .query(&[("key", api_key)]);

        if self.streaming {
            request = request.query(&[("alt", "sse")]);
        }

        let request = request.json(&body);

        Ok((request, body_string))
    }

    fn parse_response(json_text: &str) -> Result<LlmApiResponseProps, Error> {
        let response: GeminiResponse = serde_json::from_str(json_text)?;
        Ok(response.into())

    }

    async fn stream_eventsource(
        mut event_source: reqwest_eventsource::EventSource, 
        tx: Sender<Result<String, LlmStreamingError>>
    ) -> Result<LlmApiResponseProps, Error> {
        let result = tokio::spawn(async move {
            let mut stream_content = String::new();
            let mut output_tokens = 0;
            let mut input_tokens = 0;
            let mut chunks = vec![];

            while let Some(event_result) = event_source.next().await {
                match event_result {
                    Ok(event) => {
                        if let Event::Message(message) = event {
                            match serde_json::from_str::<GeminiResponseStreamChunk>(&message.data) {
                                Ok(chunk) => {
                                    if let Some(text) = chunk.candidates
                                        .first()
                                        .and_then(|c| c.content.parts.first())
                                        .map(|p| p.text.clone())
                                    {
                                        stream_content += &text;
                                        if let Err(_) = tx.send(Ok(text)).await {
                                            break;
                                        }
                                    }

                                    if let Some(usage) = &chunk.usage_metadata {
                                        // we only get the candidates_token_count on the last
                                        // message. Otherwise it just returns a running total,
                                        // so it's only worth capturing it at the end
                                        if let Some(c_toks) = usage.candidates_token_count {
                                            input_tokens += usage.prompt_token_count;
                                            output_tokens += c_toks;
                                            chunks.push(chunk); // only capturing the last chunk
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(LlmStreamingError::ParseError(e.to_string()))).await;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if e.to_string() == "Stream ended" {
                            break;
                        }

                        let _ = tx.send(Err(LlmStreamingError::StreamError(e.to_string()))).await;
                        break;
                    }
                }
            }

            let _ = tx.send(Ok("[DONE]".to_string())).await;
            event_source.close();

            let raw_response = serde_json::to_string(&chunks)
                .expect("Failed to serialize chunk to string");

            return LlmApiResponseProps {
                response_content: stream_content,
                raw_response,
                input_tokens: Some(input_tokens.into()),
                output_tokens: Some(output_tokens.into()),
                reasoning_tokens: None
            };
        }).await?;

        Ok(result)
    }

    fn create_body(&self) -> serde_json::Value {
        let system_instruction = self.props.messages.iter()
            .filter_map(|msg| match msg {
                Message::System { content } => Some(content.as_str()),
                _ => None
            })
            .collect::<Vec<&str>>()
            .join("\n\n");

        // Convert conversation history to Gemini's format
        let contents = self.props.messages.iter()
            .filter_map(|msg| match msg {
                Message::System { .. } => None,
                Message::User { content } => Some(json!({
                    "role": "user",
                    "parts": [{ "text": content }]
                })),
                Message::Assistant { content } => Some(json!({
                    "role": "model",
                    "parts": [{ "text": content }]
                })),
            })
            .collect::<Vec<_>>();

        // Build base JSON body
        let mut body = json!({
            "contents": contents
        });

        // Add system instruction if present
        if !system_instruction.is_empty() {
            body["systemInstruction"] = json!({
                "parts": [{ "text": system_instruction }]
            });
        }

        let mut generation_config = json!({
            "temperature": self.props.temperature,
            "maxOutputTokens": self.props.max_tokens
        });

        if self.props.json_mode {
            generation_config["responseMimeType"] = json!("application/json");
        } else {
            generation_config["responseMimeType"] = json!("text/plain");
        }

        body["generationConfig"] = generation_config;

        body
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_google_response_parsing() {
        let response = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "test response"
                    }]
                },
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": []
            }],
            "promptFeedback": {
                "blockReason": null,
                "safetyRatings": []
            },
            "usageMetadata": {
                "promptTokenCount": 50,
                "candidatesTokenCount": 20,
                "totalTokenCount": 70
            }
        })
        .to_string();

        let result = GeminiProvider::parse_response(&response).unwrap();
        let result: LlmApiResponseProps = result.into();
        assert_eq!(result.response_content, "test response");
        assert_eq!(result.input_tokens, Some(50));
        assert_eq!(result.output_tokens, Some(20));
    }
}
