use factory_protocol::mcp::{MCP_OAUTH_LOOPBACK_PORT, MCP_OAUTH_LOOPBACK_REDIRECT};
use std::{io, net::TcpListener, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::oneshot,
    time::timeout,
};

#[derive(serde::Serialize)]
pub(super) struct Callback {
    state: String,
    code: Option<String>,
    error: Option<&'static str>,
}

pub(super) fn bind() -> io::Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", MCP_OAUTH_LOOPBACK_PORT))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

pub(super) fn receive(
    listener: TcpListener,
    state: String,
) -> io::Result<oneshot::Receiver<Callback>> {
    let (mut sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("mcp-oauth-loopback".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                    return;
                };
                tokio::select! {
                    _ = sender.closed() => {}
                    result = timeout(Duration::from_secs(600), accept(&listener, &state)) => {
                        if let Ok(Some(callback)) = result {
                            let _ = sender.send(callback);
                        }
                    }
                }
            });
        })?;
    Ok(receiver)
}

async fn accept(listener: &tokio::net::TcpListener, state: &str) -> Option<Callback> {
    loop {
        let (mut stream, _) = listener.accept().await.ok()?;
        if let Ok(Some(callback)) =
            timeout(Duration::from_secs(2), handle(&mut stream, state)).await
        {
            return Some(callback);
        }
    }
}

async fn handle(stream: &mut TcpStream, state: &str) -> Option<Callback> {
    let mut request = Vec::new();
    {
        let mut reader = BufReader::new(&mut *stream);
        while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
            request.push(reader.read_u8().await.ok()?);
        }
    }
    let callback = request
        .ends_with(b"\r\n\r\n")
        .then(|| {
            std::str::from_utf8(&request)
                .ok()?
                .split_inclusive("\r\n")
                .next()
        })
        .flatten()
        .and_then(|line| parse(line, state));
    let (status, message) = if callback.is_some() {
        ("200 OK", crate::tr!("Rückmeldung empfangen. Weiter in YoloForge."))
    } else {
        ("400 Bad Request", crate::tr!("Keine passende Anmeldung."))
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
        message.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    callback
}

fn parse(line: &str, expected_state: &str) -> Option<Callback> {
    let mut parts = line.strip_suffix("\r\n")?.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let (path, query) = parts.next()?.split_once('?')?;
    if path != url::Url::parse(MCP_OAUTH_LOOPBACK_REDIRECT).ok()?.path()
        || !matches!(parts.next()?, "HTTP/1.1" | "HTTP/1.0")
        || parts.next().is_some()
    {
        return None;
    }
    let (mut state, mut code, mut error) = (None, None, None);
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "state" if state.is_none() => state = Some(value),
            "code" if code.is_none() && !value.is_empty() && value.len() <= 4096 => {
                code = Some(value.into_owned())
            }
            "error" if error.is_none() => error = Some("authorization_failed"),
            "state" | "code" | "error" => return None,
            _ => {}
        }
    }
    let state = state?;
    (state == expected_state && (code.is_some() ^ error.is_some())).then(|| Callback {
        state: state.into_owned(),
        code,
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_listener_ignores_wrong_state_and_delivers_the_matching_callback() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let received = receive(listener, "expected".into()).unwrap();
        for (state, status) in [("wrong", "400"), ("expected", "200")] {
            let mut browser = TcpStream::connect(address).await.unwrap();
            browser
                .write_all(
                    format!("GET /mcp-oauth/callback?state={state}&code=demo HTTP/1.1\r\nHost: {address}\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let mut reply = String::new();
            timeout(Duration::from_secs(3), browser.read_to_string(&mut reply))
                .await
                .unwrap()
                .unwrap();
            assert!(reply.starts_with(&format!("HTTP/1.1 {status}")));
            assert!(!reply.contains("code=demo"));
        }
        let callback = timeout(Duration::from_secs(3), received)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(callback.code.as_deref(), Some("demo"));
    }

    #[test]
    fn only_the_expected_callback_path_and_state_are_accepted() {
        let request = |query: &str| format!("GET /mcp-oauth/callback?{query} HTTP/1.1\r\n");
        let callback = parse(&request("state=expected&code=hello%2Bworld"), "expected").unwrap();
        assert_eq!(callback.code.as_deref(), Some("hello+world"));
        assert!(parse(&request("state=wrong&code=hello"), "expected").is_none());
        assert!(parse(
            &request("state=expected&state=other&code=hello"),
            "expected"
        )
        .is_none());
        assert!(parse(
            "GET /wrong?state=expected&code=hello HTTP/1.1\r\n",
            "expected"
        )
        .is_none());
        assert!(parse(&request("state=expected&error=access_denied"), "expected").is_some());
    }
}
