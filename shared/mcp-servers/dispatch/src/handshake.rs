//! Pre-handshake of the stdio transport: answers the requests a client sends
//! before `initialize` and hands the rest of the stream to the MCP library.
//!
//! The Antigravity CLI opens a stdio server with its own request,
//! `server/discover` (id 1), before the standard `initialize`; the MCP library
//! closes the connection when the first request is anything else. A
//! `-32601 Method not found` answer is accepted by that client, which then
//! sends `initialize` as every other client does (probe, agy 1.3.4,
//! 2026-10-14). Only the leading requests are inspected: the first line that
//! is not `server/discover` (or blank) is handed over unread.

use std::io::Cursor;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// The reader the MCP library is served from after the pre-handshake: the
/// first ordinary line, then the rest of the input.
pub type Handshaken<R> = tokio::io::Chain<Cursor<Vec<u8>>, BufReader<R>>;

/// Answers every leading `server/discover` request on `reader` with
/// `-32601 Method not found` through `writer` and returns a reader that
/// yields everything after them.
pub async fn skip_discover<R, W>(reader: R, writer: &mut W) -> std::io::Result<Handshaken<R>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut pending = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 {
            break;
        }
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        match discover_id(&line) {
            Some(id) => {
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "Method not found"},
                });
                writer.write_all(reply.to_string().as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
            None => {
                pending = line;
                break;
            }
        }
    }
    Ok(Cursor::new(pending).chain(reader))
}

/// The id of a `server/discover` request, or `None` for any other line.
fn discover_id(line: &[u8]) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_slice(line).ok()?;
    (v.get("method")?.as_str()? == "server/discover")
        .then(|| v.get("id").cloned().unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(input: &str) -> (String, String) {
        let mut out = Vec::new();
        let mut rest = skip_discover(Cursor::new(input.as_bytes().to_vec()), &mut out)
            .await
            .expect("handshake");
        let mut remaining = String::new();
        rest.read_to_string(&mut remaining)
            .await
            .expect("read rest");
        (String::from_utf8(out).expect("utf8"), remaining)
    }

    #[tokio::test]
    async fn discover_is_answered_and_initialize_is_handed_over() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{}}\n\
                     {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{}}\n\
                     {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
        let (out, rest) = run(input).await;
        let reply: serde_json::Value = serde_json::from_str(out.trim()).expect("one reply");
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(out.matches('\n').count(), 1);
        assert!(rest.starts_with("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\""));
        assert!(rest.ends_with("\"notifications/initialized\"}\n"));
    }

    #[tokio::test]
    async fn without_discover_nothing_is_written_and_nothing_is_lost() {
        let input =
            "\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\nrest\n";
        let (out, rest) = run(input).await;
        assert!(out.is_empty());
        assert_eq!(
            rest,
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\nrest\n"
        );
    }

    #[tokio::test]
    async fn several_discovers_and_a_split_read_are_handled() {
        let input = "{\"id\":1,\"method\":\"server/discover\"}\n{\"id\":\"b\",\"method\":\"server/discover\"}\n{\"id\":3,\"method\":\"initialize\"}\n";
        let (out, rest) = run(input).await;
        assert_eq!(out.matches('\n').count(), 2);
        assert!(out.contains("\"id\":1") && out.contains("\"id\":\"b\""));
        assert_eq!(rest, "{\"id\":3,\"method\":\"initialize\"}\n");
    }

    #[tokio::test]
    async fn eof_after_discover_yields_an_empty_reader() {
        let (out, rest) = run("{\"id\":1,\"method\":\"server/discover\"}\n").await;
        assert_eq!(out.matches('\n').count(), 1);
        assert!(rest.is_empty());
    }
}
