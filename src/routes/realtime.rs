use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code},
    },
    http::{HeaderMap, StatusCode, Uri, header, uri::Authority},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use futures_util::SinkExt;
use tokio::time::Instant;

use crate::{
    auth::optRememberMeUserId,
    domain::realtime::model::{EnRealtimeDelivery, StTopicSubscriptionRequest},
    error::Result,
    security,
    state::AppState,
};

// The current Java development/runtime container is Jetty 12, whose default
// maximum text, binary and frame sizes are all 64 KiB. The application does
// not override those defaults.
const I_MAX_MESSAGE_BYTES: usize = 64 * 1024;
const DUR_INITIAL_PING: Duration = Duration::from_secs(5);
const DUR_PING_INTERVAL: Duration = Duration::from_secs(60);

pub async fn websocket(
    State(stState): State<AppState>,
    oHeaders: HeaderMap,
    oJar: CookieJar,
    wsUpgrade: WebSocketUpgrade,
) -> Result<Response> {
    if !bOriginAllowed(&oHeaders, &stState.config.public_url) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    // Spring exposes a principal here only when its stateless remember-me
    // filter produced an authenticated RememberMeAuthenticationToken. Do not
    // accept the transitional `lor_session` cookie on this compatibility
    // endpoint.
    let optUserId = if let Some(stCookie) = oJar.get(security::remember_me::COOKIE_NAME) {
        optRememberMeUserId(&stState.pool, stCookie.value(), &stState.config.site_secret).await?
    } else {
        None
    };

    Ok(wsUpgrade
        .max_message_size(I_MAX_MESSAGE_BYTES)
        .max_frame_size(I_MAX_MESSAGE_BYTES)
        .on_failed_upgrade(|stError| {
            tracing::debug!(error = %stError, "WebSocket upgrade failed");
        })
        .on_upgrade(move |wsSocket| vHandleSocket(stState, wsSocket, optUserId)))
}

fn bOriginAllowed(oHeaders: &HeaderMap, sSecureUrl: &str) -> bool {
    let mut iterOrigins = oHeaders.get_all(header::ORIGIN).iter();
    let Some(stOrigin) = iterOrigins.next() else {
        // Non-browser clients without Origin pass Spring's same-origin check.
        return true;
    };
    if iterOrigins.next().is_some() {
        return false;
    }
    let Ok(sOrigin) = stOrigin.to_str() else {
        return false;
    };

    // CorsConfiguration normalizes the configured trailing slash and compares
    // origins case-insensitively. Scheme/host/port otherwise remain exact.
    let sOrigin = sOrigin.trim_end_matches('/');
    let sSecureUrl = sSecureUrl.trim_end_matches('/');
    !sOrigin.is_empty()
        && (sOrigin.eq_ignore_ascii_case(sSecureUrl) || bSameOrigin(oHeaders, sOrigin, sSecureUrl))
}

/// Spring's `OriginHandshakeInterceptor` accepts the configured allowed
/// origin *or* `WebUtils.isSameOrigin(request)`. Axum only sees HTTP behind
/// the deployment TLS proxy, so use `PUBLIC_URL` for the public request
/// scheme and the actual HTTP `Host` header for the public request authority.
/// This permits safe same-origin aliases without turning `/ws` into a
/// wildcard-origin endpoint.
fn bSameOrigin(oHeaders: &HeaderMap, sOrigin: &str, sSecureUrl: &str) -> bool {
    let mut iterHosts = oHeaders.get_all(header::HOST).iter();
    let Some(stHost) = iterHosts.next() else {
        return false;
    };
    if iterHosts.next().is_some() {
        return false;
    }
    let Ok(sHost) = stHost.to_str() else {
        return false;
    };
    let Ok(stRequestAuthority) = sHost.parse::<Authority>() else {
        return false;
    };
    let Ok(stOriginUri) = sOrigin.parse::<Uri>() else {
        return false;
    };
    let Ok(stSecureUri) = sSecureUrl.parse::<Uri>() else {
        return false;
    };

    // An Origin header is an RFC 6454 origin, never a URL with a query or a
    // non-root path. Reject such values even if their authority happens to
    // match the Host header.
    if stOriginUri.path() != "/" || stOriginUri.query().is_some() {
        return false;
    }

    let (Some(sOriginScheme), Some(sRequestScheme)) =
        (stOriginUri.scheme_str(), stSecureUri.scheme_str())
    else {
        return false;
    };
    if !matches!(sRequestScheme, "http" | "https")
        || !sOriginScheme.eq_ignore_ascii_case(sRequestScheme)
    {
        return false;
    }
    let Some(stOriginAuthority) = stOriginUri.authority() else {
        return false;
    };

    stOriginAuthority
        .host()
        .eq_ignore_ascii_case(stRequestAuthority.host())
        && optEffectivePort(sOriginScheme, stOriginAuthority)
            == optEffectivePort(sRequestScheme, &stRequestAuthority)
}

fn optEffectivePort(sScheme: &str, stAuthority: &Authority) -> Option<u16> {
    stAuthority.port_u16().or(match sScheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    })
}

fn stParseSubscription(sPayload: &str) -> std::result::Result<StTopicSubscriptionRequest, ()> {
    let mut iterParts = sPayload.splitn(2, ' ');
    let iTopicId = iterParts.next().ok_or(())?.parse::<i32>().map_err(|_| ())?;
    let iLastSeenCommentId = iterParts
        .next()
        .map_or(Ok(0), |sCommentId| sCommentId.parse::<i32>())
        .map_err(|_| ())?;
    Ok(StTopicSubscriptionRequest {
        iTopicId,
        iLastSeenCommentId,
    })
}

async fn bSendClose(wsSocket: &mut WebSocket, iCode: u16, sReason: &'static str) -> bool {
    wsSocket
        .send(Message::Close(Some(CloseFrame {
            code: iCode,
            reason: sReason.into(),
        })))
        .await
        .is_ok()
}

async fn bSendServerError(wsSocket: &mut WebSocket) -> bool {
    bSendClose(wsSocket, close_code::ERROR, "").await
}

async fn vHandleSocket(stState: AppState, mut wsSocket: WebSocket, optUserId: Option<i32>) {
    let mut stRegistration = stState.realtime.stRegisterSession(optUserId);
    let uuidSessionId = stRegistration.uuidSessionId;
    let mut tmPing = tokio::time::interval_at(Instant::now() + DUR_INITIAL_PING, DUR_PING_INTERVAL);
    tmPing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut bCloseSent = false;

    loop {
        tokio::select! {
            optIncoming = wsSocket.recv() => {
                match optIncoming {
                    Some(Ok(Message::Text(sPayload))) => {
                        let Ok(stRequest) = stParseSubscription(&sPayload) else {
                            tracing::warn!(payload = %sPayload, "invalid realtime subscription request");
                            bCloseSent = bSendServerError(&mut wsSocket).await;
                            break;
                        };
                        if let Err(stError) = stState
                            .realtime
                            .vSubscribeTopic(uuidSessionId, stRequest)
                            .await
                        {
                            tracing::warn!(
                                topic_id = stRequest.iTopicId,
                                error = %stError,
                                "realtime subscription failed"
                            );
                            bCloseSent = bSendServerError(&mut wsSocket).await;
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        // TextWebSocketHandler.handleBinaryMessage closes with
                        // CloseStatus.NOT_ACCEPTABLE and this exact reason.
                        bCloseSent = bSendClose(
                            &mut wsSocket,
                            close_code::UNSUPPORTED,
                            "Binary messages not supported",
                        )
                        .await;
                        break;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        // tungstenite automatically answers peer pings.
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(stError)) => {
                        tracing::debug!(error = %stError, "WebSocket transport closed with an error");
                        break;
                    }
                }
            }
            optDelivery = stRegistration.rxDelivery.recv() => {
                let Some(enDelivery) = optDelivery else {
                    break;
                };
                let optMessage = match enDelivery {
                    EnRealtimeDelivery::Comment(iCommentId) => {
                        match stState
                            .realtime
                            .bShouldDeliverComment(optUserId, iCommentId)
                            .await
                        {
                            Ok(true) => Some(Message::Text(format!("comment {iCommentId}").into())),
                            Ok(false) => None,
                            Err(stError) => {
                                tracing::warn!(
                                    comment_id = iCommentId,
                                    error = %stError,
                                    "failed to apply realtime ignore filter"
                                );
                                bCloseSent = bSendServerError(&mut wsSocket).await;
                                break;
                            }
                        }
                    }
                    EnRealtimeDelivery::EventsRefresh => {
                        Some(Message::Text("events-refresh".into()))
                    }
                };
                if let Some(stMessage) = optMessage
                    && let Err(stError) = wsSocket.send(stMessage).await
                {
                    tracing::debug!(error = %stError, "failed to send realtime message");
                    break;
                }
            }
            _ = tmPing.tick() => {
                if let Err(stError) = wsSocket.send(Message::Ping(Vec::new().into())).await {
                    tracing::debug!(error = %stError, "failed to send WebSocket keepalive");
                    break;
                }
            }
        }
    }

    stState.realtime.vUnregisterSession(uuidSessionId);
    if !bCloseSent {
        let _ = wsSocket.close().await;
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{Router, http::HeaderValue, routing::get};
    use sha2::{Digest, Sha256};
    use sqlx::postgres::PgPoolOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::task::JoinHandle;
    use tower_http::services::ServeDir;

    use super::*;

    async fn stTestServer() -> (SocketAddr, JoinHandle<()>) {
        let oListener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stAddress = oListener.local_addr().unwrap();
        let oPool = PgPoolOptions::new()
            .connect_lazy("postgres://linuxweb:linuxweb@127.0.0.1:1/unused")
            .unwrap();
        let stState = AppState::new(
            crate::config::Config {
                host: "127.0.0.1".to_string(),
                port: stAddress.port(),
                database_url: "postgres://linuxweb:linuxweb@127.0.0.1:1/unused".to_string(),
                public_url: format!("http://{stAddress}"),
                ws_url: format!("ws://{stAddress}/"),
                static_dir: "static".to_string(),
                upload_dir: "uploads".to_string(),
                site_secret: "unused-test-secret".to_string(),
                opensearch_url: None,
                captcha_public_key: None,
                captcha_private_key: None,
                captcha_verify_url: "https://hcaptcha.com/siteverify".to_owned(),
                admin_email: None,
                smtp_host: "localhost".to_owned(),
                smtp_port: 25,
                smtp_helo_name: "localhost".to_owned(),
                telegram_token: None,
                fallback_proxy_url: None,
                enable_background_jobs: false,
                clean_old_userpics: false,
                trusted_proxy_cidrs: Vec::new(),
                page_size: 30,
                enable_hsts: false,
                enable_dev_bypasses: false,
            },
            oPool,
        );
        let cApp = Router::new()
            .route("/ws", get(websocket))
            .nest_service(
                "/webjars",
                ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/static/webjars")),
            )
            .with_state(stState);
        let hServer = tokio::spawn(async move {
            axum::serve(oListener, cApp).await.unwrap();
        });
        (stAddress, hServer)
    }

    async fn stHandshake(stAddress: SocketAddr, sOrigin: &str) -> (tokio::net::TcpStream, String) {
        let mut tcpClient = tokio::net::TcpStream::connect(stAddress).await.unwrap();
        let sRequest = format!(
            "GET /ws HTTP/1.1\r\nHost: {stAddress}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nOrigin: {sOrigin}\r\n\r\n"
        );
        tcpClient.write_all(sRequest.as_bytes()).await.unwrap();

        let mut vecResponse = Vec::new();
        while !vecResponse.ends_with(b"\r\n\r\n") {
            let mut arrByte = [0_u8; 1];
            tcpClient.read_exact(&mut arrByte).await.unwrap();
            vecResponse.push(arrByte[0]);
        }
        (
            tcpClient,
            String::from_utf8(vecResponse).expect("HTTP headers are ASCII"),
        )
    }

    async fn stReadServerFrame(tcpClient: &mut tokio::net::TcpStream) -> (u8, Vec<u8>) {
        let mut arrHeader = [0_u8; 2];
        tcpClient.read_exact(&mut arrHeader).await.unwrap();
        assert_eq!(arrHeader[0] & 0x80, 0x80, "server frame must have FIN");
        assert_eq!(arrHeader[1] & 0x80, 0, "server frames are not masked");
        let iLength = match arrHeader[1] & 0x7f {
            iLength @ 0..=125 => usize::from(iLength),
            126 => {
                let mut arrLength = [0_u8; 2];
                tcpClient.read_exact(&mut arrLength).await.unwrap();
                usize::from(u16::from_be_bytes(arrLength))
            }
            127 => {
                let mut arrLength = [0_u8; 8];
                tcpClient.read_exact(&mut arrLength).await.unwrap();
                usize::try_from(u64::from_be_bytes(arrLength)).unwrap()
            }
            _ => unreachable!("the WebSocket length field is masked to seven bits"),
        };
        let mut vecPayload = vec![0_u8; iLength];
        tcpClient.read_exact(&mut vecPayload).await.unwrap();
        (arrHeader[0] & 0x0f, vecPayload)
    }

    async fn vSendMaskedFrame(
        tcpClient: &mut tokio::net::TcpStream,
        iOpcode: u8,
        arrPayload: &[u8],
    ) {
        let arrMask = [0x11_u8, 0x22, 0x33, 0x44];
        assert!(arrPayload.len() <= 125);
        let mut vecFrame = vec![0x80 | iOpcode, 0x80 | (arrPayload.len() as u8)];
        vecFrame.extend_from_slice(&arrMask);
        vecFrame.extend(
            arrPayload
                .iter()
                .enumerate()
                .map(|(iIndex, iByte)| iByte ^ arrMask[iIndex % arrMask.len()]),
        );
        tcpClient.write_all(&vecFrame).await.unwrap();
    }

    async fn vSendMaskedText(tcpClient: &mut tokio::net::TcpStream, sPayload: &str) {
        vSendMaskedFrame(tcpClient, 0x01, sPayload.as_bytes()).await;
    }

    #[test]
    fn parses_original_client_protocol() {
        assert_eq!(
            stParseSubscription("42"),
            Ok(StTopicSubscriptionRequest {
                iTopicId: 42,
                iLastSeenCommentId: 0,
            })
        );
        assert_eq!(
            stParseSubscription("42 9001"),
            Ok(StTopicSubscriptionRequest {
                iTopicId: 42,
                iLastSeenCommentId: 9001,
            })
        );
        assert_eq!(
            stParseSubscription("+42 -1"),
            Ok(StTopicSubscriptionRequest {
                iTopicId: 42,
                iLastSeenCommentId: -1,
            })
        );
    }

    #[test]
    fn rejects_payloads_java_to_int_rejects() {
        for sPayload in ["", " 42", "42 ", "42  1", "42 1 2", "2147483648"] {
            assert!(
                stParseSubscription(sPayload).is_err(),
                "payload should fail: {sPayload:?}"
            );
        }
    }

    #[test]
    fn origin_matches_configured_secure_url() {
        let mut oHeaders = HeaderMap::new();
        assert!(bOriginAllowed(&oHeaders, "https://www.linux.org.ru/"));

        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://www.linux.org.ru"),
        );
        assert!(bOriginAllowed(&oHeaders, "https://www.linux.org.ru/"));
        assert!(bOriginAllowed(&oHeaders, "HTTPS://WWW.LINUX.ORG.RU"));
        assert!(!bOriginAllowed(&oHeaders, "http://www.linux.org.ru/"));
        assert!(!bOriginAllowed(&oHeaders, "https://www.linux.org.ru:8443/"));
    }

    #[test]
    fn origin_accepts_only_real_same_origin_host_aliases() {
        let mut oHeaders = HeaderMap::new();
        oHeaders.insert(header::HOST, HeaderValue::from_static("lor-alias.example"));
        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://lor-alias.example"),
        );
        assert!(bOriginAllowed(&oHeaders, "https://canonical.example/"));

        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://lor-alias.example"),
        );
        assert!(!bOriginAllowed(&oHeaders, "https://canonical.example/"));
        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://lor-alias.example:8443"),
        );
        assert!(!bOriginAllowed(&oHeaders, "https://canonical.example/"));
        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://lor-alias.example/not-an-origin"),
        );
        assert!(!bOriginAllowed(&oHeaders, "https://canonical.example/"));
    }

    #[test]
    fn origin_rejects_multiple_empty_and_prefix_attacks() {
        let mut oHeaders = HeaderMap::new();
        oHeaders.insert(header::ORIGIN, HeaderValue::from_static(""));
        assert!(!bOriginAllowed(&oHeaders, "https://www.linux.org.ru/"));

        oHeaders.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://www.linux.org.ru.evil.example"),
        );
        assert!(!bOriginAllowed(&oHeaders, "https://www.linux.org.ru/"));

        oHeaders.append(
            header::ORIGIN,
            HeaderValue::from_static("https://www.linux.org.ru"),
        );
        assert!(!bOriginAllowed(&oHeaders, "https://www.linux.org.ru/"));
    }

    #[tokio::test]
    async fn websocket_handshake_and_invalid_protocol_close_match_java() {
        let (stAddress, hServer) = stTestServer().await;
        let (mut tcpClient, sResponse) =
            stHandshake(stAddress, &format!("http://{stAddress}")).await;
        assert!(sResponse.starts_with("HTTP/1.1 101 "));

        vSendMaskedText(&mut tcpClient, "not-a-topic").await;
        let (iOpcode, vecPayload) = stReadServerFrame(&mut tcpClient).await;
        assert_eq!(iOpcode, 0x08);
        assert_eq!(vecPayload, close_code::ERROR.to_be_bytes());
        hServer.abort();
    }

    #[tokio::test]
    async fn websocket_rejects_binary_with_spring_text_handler_status() {
        let (stAddress, hServer) = stTestServer().await;
        let (mut tcpClient, sResponse) =
            stHandshake(stAddress, &format!("http://{stAddress}")).await;
        assert!(sResponse.starts_with("HTTP/1.1 101 "));

        vSendMaskedFrame(&mut tcpClient, 0x02, b"binary").await;
        let (iOpcode, vecPayload) = stReadServerFrame(&mut tcpClient).await;
        assert_eq!(iOpcode, 0x08);
        assert_eq!(
            &vecPayload[..2],
            close_code::UNSUPPORTED.to_be_bytes().as_slice()
        );
        assert_eq!(&vecPayload[2..], b"Binary messages not supported");
        hServer.abort();
    }

    #[tokio::test]
    async fn websocket_rejects_wrong_origin_before_upgrade() {
        let (stAddress, hServer) = stTestServer().await;
        let (_, sResponse) = stHandshake(stAddress, "https://evil.example").await;
        assert!(sResponse.starts_with("HTTP/1.1 403 "));
        hServer.abort();
    }

    #[tokio::test]
    async fn browser_webjar_path_serves_the_exact_original_jquery() {
        const ARR_JQUERY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/webjars/jquery/3.7.1/jquery.min.js"
        ));
        let (stAddress, hServer) = stTestServer().await;
        let stResponse = reqwest::get(format!(
            "http://{stAddress}/webjars/jquery/3.7.1/jquery.min.js"
        ))
        .await
        .expect("vendored browser dependency must be reachable");
        assert_eq!(stResponse.status(), StatusCode::OK);
        assert!(
            stResponse
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|stValue| stValue.to_str().ok())
                .is_some_and(|sValue| sValue.starts_with("text/javascript"))
        );
        let arrServed = stResponse.bytes().await.expect("jQuery response body");
        assert_eq!(arrServed.as_ref(), ARR_JQUERY);
        assert_eq!(ARR_JQUERY.len(), 87_533);
        assert_eq!(
            Sha256::digest(ARR_JQUERY)
                .iter()
                .map(|iByte| format!("{iByte:02x}"))
                .collect::<String>(),
            "fc9a93dd241f6b045cbff0481cf4e1901becd0e12fb45166a8f17f95823f0b1a"
        );
        hServer.abort();
    }

    #[tokio::test]
    async fn websocket_sends_java_keepalive_ping_after_five_seconds() {
        let (stAddress, hServer) = stTestServer().await;
        let (mut tcpClient, sResponse) =
            stHandshake(stAddress, &format!("http://{stAddress}")).await;
        assert!(sResponse.starts_with("HTTP/1.1 101 "));
        let stFrame =
            tokio::time::timeout(Duration::from_secs(6), stReadServerFrame(&mut tcpClient))
                .await
                .expect("keepalive was not sent after five seconds");
        assert_eq!(stFrame, (0x09, Vec::new()));
        hServer.abort();
    }
}
