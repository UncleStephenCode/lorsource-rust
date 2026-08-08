//! Java-compatible adapter for `GeoLocationService` / ipwho.is.

use serde::Deserialize;

const S_IPWHOIS_BASE_URL: &str = "http://ipwho.is";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StGeoLocation {
    pub optCountry: Option<String>,
    pub optRegion: Option<String>,
    pub optCity: Option<String>,
    pub optOrganization: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StIpWhoIsConnection {
    org: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StIpWhoIsResponse {
    success: bool,
    message: Option<String>,
    country: Option<String>,
    region: Option<String>,
    city: Option<String>,
    connection: Option<StIpWhoIsConnection>,
}

#[derive(Debug, Clone)]
pub struct CGeoLocationService {
    cHttp: reqwest::Client,
    sBaseUrl: String,
}

impl CGeoLocationService {
    pub fn new(cHttp: reqwest::Client) -> Self {
        Self::stWithBaseUrl(cHttp, S_IPWHOIS_BASE_URL)
    }

    fn stWithBaseUrl(cHttp: reqwest::Client, sBaseUrl: impl Into<String>) -> Self {
        Self {
            cHttp,
            sBaseUrl: sBaseUrl.into().trim_end_matches('/').to_owned(),
        }
    }

    pub async fn stGetLocation(&self, sIp: &str) -> Result<StGeoLocation, String> {
        let sUrl = format!("{}/{}", self.sBaseUrl, urlencoding::encode(sIp));
        let stResponse = self
            .cHttp
            .get(sUrl)
            .send()
            .await
            .map_err(|stError| format!("Request error: {stError}"))?;
        let bSuccessStatus = stResponse.status().is_success();
        let sBody = stResponse
            .text()
            .await
            .map_err(|stError| format!("Request error: {stError}"))?;

        // sttp's default `asString` response is `Left(body)` for a non-2xx
        // status. Java consequently returns a request error without trying to
        // decode an error page as the ipwho.is JSON contract.
        if !bSuccessStatus {
            return Err(format!("Request error: {sBody}"));
        }

        let stResponse: StIpWhoIsResponse =
            serde_json::from_str(&sBody).map_err(|stError| format!("Parse error: {stError}"))?;
        if !stResponse.success {
            return Err(stResponse
                .message
                .unwrap_or_else(|| "Unknown error".to_owned()));
        }

        Ok(StGeoLocation {
            optCountry: stResponse.country,
            optRegion: stResponse.region,
            optCity: stResponse.city,
            optOrganization: stResponse.connection.and_then(|stValue| stValue.org),
        })
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn stServiceForResponse(
        sStatus: &str,
        sBody: &str,
    ) -> (CGeoLocationService, tokio::task::JoinHandle<String>) {
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let stAddress = stListener.local_addr().expect("listener address");
        let sStatus = sStatus.to_owned();
        let sBody = sBody.to_owned();
        let hServer = tokio::spawn(async move {
            let (mut stStream, _) = stListener.accept().await.expect("test request");
            let mut vecRequest = vec![0_u8; 4096];
            let iRead = stStream.read(&mut vecRequest).await.expect("read request");
            let sRequest = String::from_utf8_lossy(&vecRequest[..iRead]).to_string();
            let sResponse = format!(
                "HTTP/1.1 {sStatus}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sBody}",
                sBody.len()
            );
            stStream
                .write_all(sResponse.as_bytes())
                .await
                .expect("write response");
            sRequest
        });
        (
            CGeoLocationService::stWithBaseUrl(
                reqwest::Client::new(),
                format!("http://{stAddress}"),
            ),
            hServer,
        )
    }

    #[tokio::test]
    async fn successful_response_matches_java_projection() {
        let (cService, hServer) = stServiceForResponse(
            "200 OK",
            r#"{"success":true,"country":"US","region":"Virginia","city":"Ashburn","connection":{"org":"Example ASN"}}"#,
        )
        .await;

        let stLocation = cService
            .stGetLocation("8.8.8.8")
            .await
            .expect("successful location");
        assert_eq!(
            stLocation,
            StGeoLocation {
                optCountry: Some("US".to_owned()),
                optRegion: Some("Virginia".to_owned()),
                optCity: Some("Ashburn".to_owned()),
                optOrganization: Some("Example ASN".to_owned()),
            }
        );
        assert!(hServer.await.unwrap().starts_with("GET /8.8.8.8 HTTP/1.1"));
    }

    #[tokio::test]
    async fn api_and_http_errors_follow_java_either_contract() {
        let (cApiService, hApiServer) = stServiceForResponse(
            "200 OK",
            r#"{"success":false,"message":"Invalid IP address"}"#,
        )
        .await;
        assert_eq!(
            cApiService.stGetLocation("bad ip").await,
            Err("Invalid IP address".to_owned())
        );
        assert!(
            hApiServer
                .await
                .unwrap()
                .starts_with("GET /bad%20ip HTTP/1.1")
        );

        let (cHttpService, hHttpServer) =
            stServiceForResponse("429 Too Many Requests", "rate limited").await;
        assert_eq!(
            cHttpService.stGetLocation("8.8.4.4").await,
            Err("Request error: rate limited".to_owned())
        );
        hHttpServer.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_success_body_is_a_parse_error() {
        let (cService, hServer) = stServiceForResponse("200 OK", "not-json").await;
        let sError = cService
            .stGetLocation("1.1.1.1")
            .await
            .expect_err("invalid JSON must fail");
        assert!(sError.starts_with("Parse error:"));
        hServer.await.unwrap();
    }
}
