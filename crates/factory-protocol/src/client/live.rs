use super::*;
pub struct EventFeed {
    response: reqwest::Response,
    buffer: Vec<u8>,
    cursor: u64,
}
impl FactoryClient {
    pub async fn event_stream(&self, after: u64) -> Result<EventFeed, ClientError> {
        self.feed_at("/v1/events/stream", after).await
    }
    pub async fn desktop_event_stream(&self, after: u64) -> Result<EventFeed, ClientError> {
        self.feed_at("/v1/desktop/events/stream", after).await
    }
    async fn feed_at(&self, path: &str, after: u64) -> Result<EventFeed, ClientError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("last-event-id", after.to_string())
            .timeout(Duration::from_secs(3600))
            .send()
            .await
            .map_err(|_| ClientError::Transport)?;
        if !response.status().is_success() {
            let status = response.status();
            let error = response.json::<ApiError>().await.ok();
            return Err(ClientError::Http {
                status,
                code: error.as_ref().map(|e| e.code.clone()),
                current_revision: error.and_then(|e| e.current_revision),
            });
        }
        Ok(EventFeed {
            response,
            buffer: Vec::new(),
            cursor: after,
        })
    }
}
fn frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    let (index, len) = lf.into_iter().chain(crlf).min_by_key(|v| v.0)?;
    let value = buffer[..index].to_vec();
    buffer.drain(..index + len);
    Some(value)
}
fn parse(bytes: &[u8], cursor: u64) -> Result<Option<FactoryEvent>, ClientError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ClientError::InvalidResponse)?;
    let mut id = None;
    let mut data = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("id:") {
            id = value.trim().parse::<u64>().ok();
        }
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let event: FactoryEvent =
        serde_json::from_str(&data.join("\n")).map_err(|_| ClientError::InvalidResponse)?;
    if id != Some(event.sequence) || event.sequence <= cursor {
        return Err(ClientError::InvalidResponse);
    }
    Ok(Some(event))
}
impl EventFeed {
    pub async fn next(&mut self) -> Result<Option<FactoryEvent>, ClientError> {
        loop {
            if let Some(bytes) = frame(&mut self.buffer) {
                if let Some(event) = parse(&bytes, self.cursor)? {
                    self.cursor = event.sequence;
                    return Ok(Some(event));
                }
                continue;
            }
            let Some(chunk) = self
                .response
                .chunk()
                .await
                .map_err(|_| ClientError::Transport)?
            else {
                return if self.buffer.is_empty() {
                    Ok(None)
                } else {
                    Err(ClientError::InvalidResponse)
                };
            };
            if self.buffer.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(ClientError::InvalidResponse);
            }
            self.buffer.extend_from_slice(&chunk);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_handle_split_crlf_unicode_heartbeats_and_invalid_replay() {
        let event = FactoryEvent {
            sequence: 2,
            kind: "test".into(),
            aggregate_id: "a".into(),
            created_unix_ms: 1,
            payload: serde_json::json!({"text":"Grüße"}),
        };
        let mut bytes = b": keepalive\r\n\r".to_vec();
        assert!(frame(&mut bytes).is_none());
        bytes.extend_from_slice(b"\n");
        assert!(parse(&frame(&mut bytes).unwrap(), 1).unwrap().is_none());
        bytes.extend_from_slice(
            format!(
                "id: 2\r\ndata: {}\r\n\r\n",
                serde_json::to_string(&event).unwrap()
            )
            .as_bytes(),
        );
        let value = frame(&mut bytes).unwrap();
        assert_eq!(parse(&value, 1).unwrap().unwrap().sequence, 2);
        assert!(parse(&value, 2).is_err());
        assert!(parse(b"id: 3\ndata: {}", 1).is_err());
    }
}
