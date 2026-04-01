//! Common SSE (Server-Sent Events) parsing utilities shared by providers.

use futures::stream::Stream;
use std::pin::Pin;

/// SSE stream that buffers bytes and yields parsed events.
/// Used by Anthropic and OpenAI providers for streaming responses.
pub struct SseStream<F> {
    byte_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buffer: String,
    parser: F,
    done: bool,
}

impl<F, T> SseStream<F>
where
    F: Fn(&str) -> Option<T>,
{
    pub fn new(
        byte_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
        parser: F,
    ) -> Self {
        Self {
            byte_stream,
            buffer: String::new(),
            parser,
            done: false,
        }
    }
}

impl<F, T> Stream for SseStream<F>
where
    F: Fn(&str) -> Option<T> + Unpin,
    T: Unpin,
{
    type Item = T;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.done {
            return std::task::Poll::Ready(None);
        }

        loop {
            // Check buffer for complete SSE event (delimited by \n\n)
            if let Some(pos) = this.buffer.find("\n\n") {
                let event_text = this.buffer[..pos].to_string();
                this.buffer = this.buffer[pos + 2..].to_string();

                if let Some(parsed) = (this.parser)(&event_text) {
                    return std::task::Poll::Ready(Some(parsed));
                }
                continue;
            }

            // Read more bytes from the underlying stream
            match this.byte_stream.as_mut().poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => {
                    this.buffer.push_str(&String::from_utf8_lossy(&bytes));
                }
                std::task::Poll::Ready(Some(Err(_))) | std::task::Poll::Ready(None) => {
                    this.done = true;
                    if !this.buffer.trim().is_empty() {
                        let remaining = std::mem::take(&mut this.buffer);
                        if let Some(parsed) = (this.parser)(&remaining) {
                            return std::task::Poll::Ready(Some(parsed));
                        }
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_sse_stream_parses_events() {
        let data = b"event: message\ndata: hello\n\nevent: message\ndata: world\n\n";
        let byte_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
            Box::pin(stream::once(async { Ok(bytes::Bytes::from_static(data)) }));

        let stream = SseStream::new(byte_stream, |event_text: &str| {
            for line in event_text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    return Some(data.to_string());
                }
            }
            None
        });

        let results: Vec<String> = stream.collect().await;
        assert_eq!(results, vec!["hello", "world"]);
    }

    #[tokio::test]
    async fn test_sse_stream_handles_chunked_data() {
        let chunks = vec![
            Ok(bytes::Bytes::from_static(b"data: hel")),
            Ok(bytes::Bytes::from_static(b"lo\n\ndata: wor")),
            Ok(bytes::Bytes::from_static(b"ld\n\n")),
        ];
        let byte_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
            Box::pin(stream::iter(chunks));

        let stream = SseStream::new(byte_stream, |event_text: &str| {
            for line in event_text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    return Some(data.to_string());
                }
            }
            None
        });

        let results: Vec<String> = stream.collect().await;
        assert_eq!(results, vec!["hello", "world"]);
    }

    #[tokio::test]
    async fn test_sse_stream_skips_unparseable_events() {
        let data = b"event: ping\ndata: {}\n\nevent: message\ndata: real\n\n";
        let byte_stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
            Box::pin(stream::once(async { Ok(bytes::Bytes::from_static(data)) }));

        let stream = SseStream::new(byte_stream, |event_text: &str| {
            for line in event_text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data != "{}" {
                        return Some(data.to_string());
                    }
                }
            }
            None
        });

        let results: Vec<String> = stream.collect().await;
        assert_eq!(results, vec!["real"]);
    }
}
