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

pub struct OpenaiProvider<'a> {
    props: &'a LlmProps,
    streaming: bool
}


impl<'a> OpenaiProvider<'a> {
    pub fn new(props: &'a LlmProps, streaming: bool) -> Self {
        OpenaiProvider {
            props,
            streaming
        }
    }
}


#[derive(Deserialize, Serialize, Clone)]
struct OpenaiResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    system_fingerprint: String,
    choices: Vec<OpenaiResponseChoice>,
    usage: Option<OpenaiUsage>,
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenaiResponseChoice {
    index: i32,
    message: OpenaiMessageContent,
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenaiMessageContent {
    role: String,
    content: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenaiUsage {
    prompt_tokens: i32,
    completion_tokens: i32,
    total_tokens: i32,
}

// STREAMING RESPONSE
#[derive(Deserialize, Serialize, Clone)]
struct OpenAiResponseStreamChunk {
    choices: Vec<OpenAiResponseStreamChoice>,
    usage: Option<OpenAiResponseStreamUsage>
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenAiResponseStreamChoice {
    delta: OpenAiResponseStreamDelta,
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenAiResponseStreamDelta {
    content: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
struct OpenAiResponseStreamUsage {
    completion_tokens: i64,
    prompt_tokens: i64,
    total_tokens: i64,
}

impl From<OpenaiResponse> for LlmApiResponseProps {
    fn from(response: OpenaiResponse) -> Self {
        // assuming we want the first choice's content, if it exists
        let response_content = response
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .unwrap_or_default();

        // we'll serialize the full response back to a string for raw_response
        let raw_response = serde_json::to_string(&response).unwrap_or_default();

        let input_tokens = response.usage.as_ref().map(|u| u.prompt_tokens as i64);
        let output_tokens = response.usage.as_ref().map(|u| u.completion_tokens as i64);

        LlmApiResponseProps {
            response_content,
            raw_response,
            input_tokens,
            output_tokens,
            reasoning_tokens: None
        }
    }
}

impl<'a> LlmProvider for OpenaiProvider<'a> {
    fn build_request(&self) -> Result<(RequestBuilder, String), Error> {
        let client = reqwest::Client::new();
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| Error::Auth)?;

        let body = self.create_body();
        let body_string = body.to_string();

        println!("body: {}", body);

        let request = client
            .post(format!("{}/chat/completions", &self.props.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body);

        Ok((request, body_string))
    }

    fn parse_response(json_text: &str) -> Result<LlmApiResponseProps, Error> {
        let response: OpenaiResponse = serde_json::from_str(json_text)?;
        Ok(response.into())
    }

    async fn stream_eventsource(
        mut event_source: reqwest_eventsource::EventSource, 
        tx: Sender<Result<String, LlmStreamingError>>
    ) -> Result<LlmApiResponseProps, Error> {
        println!("stream_eventsourece invoked");

        let result = tokio::spawn(async move {
            let mut stream_content = String::new();
            let mut output_tokens = 0;
            let mut input_tokens = 0;
            let mut chunks = vec![];

            while let Some(event_result) = event_source.next().await {
                match event_result {
                    Ok(event) => {
                        if let Event::Message(message) = event {
                            if message.data == "[DONE]" {
                                let _ = tx.send(Ok("[DONE]".to_string())).await;
                                break;
                            }

                            match serde_json::from_str::<OpenAiResponseStreamChunk>(&message.data) {
                                Ok(chunk) => {
                                    if let Some(content) = chunk.choices
                                        .first()
                                        .and_then(|c| c.delta.content.as_ref())
                                    {
                                        stream_content += &content.clone();
                                        if let Err(_) = tx.send(Ok(content.to_string())).await {
                                            break;
                                        }
                                    }

                                    if let Some(usage) = &chunk.usage {
                                        output_tokens = usage.completion_tokens;
                                        input_tokens = usage.prompt_tokens;
                                        chunks.push(chunk); // only push last chunk
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
                        let _ = tx.send(Err(LlmStreamingError::StreamError(e.to_string()))).await;
                        break;
                    }
                }
            }
            
            event_source.close();

            let raw_response = serde_json::to_string(&chunks)
                .expect("Failed to serialize chunk to string");

            return LlmApiResponseProps {
                response_content: stream_content,
                raw_response,
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                reasoning_tokens: None
            };
        }).await?;

        Ok(result)
    }

    fn create_body(&self) -> serde_json::Value {
        let messages = self.props.messages.iter()
            .map(|msg| match msg {
                Message::System { content } => json!({
                    "role": "system",
                    "content": content
                }),
                Message::User { content } => json!({
                    "role": "user",
                    "content": content
                }),
                Message::Assistant { content } => json!({
                    "role": "assistant",
                    "content": content
                }),
            })
            .collect::<Vec<_>>();

        let mut body = json!({
            "model": self.props.model_name,
            "messages": messages,
            "temperature": self.props.temperature,
            "max_completion_tokens": self.props.max_tokens
        });

        if self.streaming {
            body["stream"] = serde_json::Value::Bool(true);
            body["stream_options"] = json!({
                "include_usage": true
            });
        }

        if self.props.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }

        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Unit tests for response parsing
    #[test]
    fn test_openai_response_parsing() {
        let response = json!({
            "id": "cmpl-some-id",
            "object": "chat.completion",
            "created": 1678901234,
            "model": "gpt-3.5-turbo",
            "system_fingerprint": "fp_some_fingerprint",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "test response"
                }
            }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 20,
                "total_tokens": 70
            }
        })
        .to_string();

        let result = OpenaiProvider::parse_response(&response).unwrap();
        let result: LlmApiResponseProps = result.into();
        assert_eq!(result.response_content, "test response");
        assert_eq!(result.input_tokens, Some(50));
        assert_eq!(result.output_tokens, Some(20));
    }

}
