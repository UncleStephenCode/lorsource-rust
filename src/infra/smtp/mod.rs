use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};

use crate::{
    domain::{
        email::address::{StStrictInternetAddress, optParseStrictInternetAddress},
        email::model::StEmailMessage,
        email::repository::TrEmailSender,
    },
    error::{AppError, Result},
};

const DT_SMTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct CSmtpEmailSender {
    sHost: String,
    iPort: u16,
    sHeloName: String,
}

impl CSmtpEmailSender {
    pub fn new(sHost: impl Into<String>, iPort: u16, sHeloName: impl Into<String>) -> Self {
        Self {
            sHost: sHost.into(),
            iPort,
            sHeloName: sHeloName.into(),
        }
    }

    async fn vSendCommand<W>(
        oWriter: &mut W,
        sCommand: &str,
    ) -> std::result::Result<(), std::io::Error>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        oWriter.write_all(sCommand.as_bytes()).await?;
        oWriter.write_all(b"\r\n").await?;
        oWriter.flush().await
    }
}

#[async_trait]
impl TrEmailSender for CSmtpEmailSender {
    async fn vSend(&self, stMessage: &StEmailMessage) -> Result<()> {
        let stFrom = stValidateMailbox(&stMessage.sFrom)?;
        let stTo = stValidateMailbox(&stMessage.sTo)?;
        vValidateHeader(&stMessage.sSubject)?;
        vValidateSmtpAtom(&self.sHeloName, "SMTP_HELO_NAME")?;

        let oStream = timeout(
            DT_SMTP_TIMEOUT,
            TcpStream::connect((self.sHost.as_str(), self.iPort)),
        )
        .await
        .map_err(|_| AppError::Anyhow(anyhow::anyhow!("SMTP connection timed out")))??;
        let (oReadHalf, mut oWriteHalf) = oStream.into_split();
        let mut oReader = BufReader::new(oReadHalf);

        vExpectResponse(&mut oReader, &[220]).await?;
        Self::vSendCommand(&mut oWriteHalf, &format!("HELO {}", self.sHeloName)).await?;
        vExpectResponse(&mut oReader, &[250]).await?;
        Self::vSendCommand(&mut oWriteHalf, &format!("MAIL FROM:<{}>", stFrom.sAddress)).await?;
        vExpectResponse(&mut oReader, &[250]).await?;
        Self::vSendCommand(&mut oWriteHalf, &format!("RCPT TO:<{}>", stTo.sAddress)).await?;
        vExpectRecipientResponse(&mut oReader).await?;
        Self::vSendCommand(&mut oWriteHalf, "DATA").await?;
        vExpectResponse(&mut oReader, &[354]).await?;

        let sWireMessage = sWireMessage(stMessage)?;
        oWriteHalf.write_all(sWireMessage.as_bytes()).await?;
        if !sWireMessage.ends_with("\r\n") {
            oWriteHalf.write_all(b"\r\n").await?;
        }
        oWriteHalf.write_all(b".\r\n").await?;
        oWriteHalf.flush().await?;
        vExpectResponse(&mut oReader, &[250]).await?;

        Self::vSendCommand(&mut oWriteHalf, "QUIT").await?;
        let _ = vExpectResponse(&mut oReader, &[221]).await;
        Ok(())
    }
}

fn stValidateMailbox(sMailbox: &str) -> Result<StStrictInternetAddress> {
    optParseStrictInternetAddress(sMailbox)
        .ok_or_else(|| AppError::BadRequest("Incorrect email address".to_string()))
}

fn vValidateHeader(sValue: &str) -> Result<()> {
    if sValue.chars().any(char::is_control) {
        return Err(AppError::BadRequest("invalid email header".to_string()));
    }
    Ok(())
}

fn vValidateSmtpAtom(sValue: &str, sName: &str) -> Result<()> {
    if sValue.is_empty()
        || !sValue.chars().all(|cCharacter| {
            cCharacter.is_ascii_alphanumeric() || matches!(cCharacter, '.' | '-' | ':' | '[' | ']')
        })
    {
        return Err(AppError::BadRequest(format!("invalid {sName}")));
    }
    Ok(())
}

fn sEncodedHeader(sValue: &str) -> String {
    if sValue.is_ascii() {
        sValue.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", STANDARD.encode(sValue.as_bytes()))
    }
}

fn sWireMessage(stMessage: &StEmailMessage) -> Result<String> {
    let stFrom = stValidateMailbox(&stMessage.sFrom)?;
    let stTo = stValidateMailbox(&stMessage.sTo)?;
    vValidateHeader(&stMessage.sSubject)?;
    let mut sResult = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@linux.org.ru>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n",
        stFrom.sAddress,
        stTo.sAddress,
        sEncodedHeader(&stMessage.sSubject),
        chrono::Utc::now().to_rfc2822(),
        uuid::Uuid::new_v4(),
    );
    let sNormalized = stMessage.sBody.replace("\r\n", "\n").replace('\r', "\n");
    for (iIndex, sLine) in sNormalized.split('\n').enumerate() {
        if iIndex > 0 {
            sResult.push_str("\r\n");
        }
        if sLine.starts_with('.') {
            sResult.push('.');
        }
        sResult.push_str(sLine);
    }
    Ok(sResult)
}

#[derive(Debug)]
struct StSmtpResponse {
    iCode: u16,
    sLastLine: String,
}

async fn stReadResponse<R>(oReader: &mut R) -> Result<StSmtpResponse>
where
    R: AsyncBufRead + Unpin,
{
    let mut sLastLine = String::new();
    loop {
        sLastLine.clear();
        let iRead = timeout(DT_SMTP_TIMEOUT, oReader.read_line(&mut sLastLine))
            .await
            .map_err(|_| AppError::Anyhow(anyhow::anyhow!("SMTP response timed out")))??;
        if iRead == 0 {
            return Err(AppError::Anyhow(anyhow::anyhow!(
                "SMTP server closed the connection"
            )));
        }
        let iCode = sLastLine
            .get(..3)
            .and_then(|sCode| sCode.parse::<u16>().ok())
            .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("invalid SMTP response")))?;
        let bMore = sLastLine.as_bytes().get(3) == Some(&b'-');
        if !bMore {
            return Ok(StSmtpResponse {
                iCode,
                sLastLine: sLastLine.trim_end().to_owned(),
            });
        }
    }
}

async fn vExpectResponse<R>(oReader: &mut R, vecExpected: &[u16]) -> Result<()>
where
    R: AsyncBufRead + Unpin,
{
    let stResponse = stReadResponse(oReader).await?;
    if !vecExpected.contains(&stResponse.iCode) {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "SMTP command failed with status {}: {}",
            stResponse.iCode,
            stResponse.sLastLine
        )));
    }
    Ok(())
}

async fn vExpectRecipientResponse<R>(oReader: &mut R) -> Result<()>
where
    R: AsyncBufRead + Unpin,
{
    let stResponse = stReadResponse(oReader).await?;
    if ![250, 251].contains(&stResponse.iCode) {
        return Err(AppError::SmtpAddressRejected {
            iStatus: stResponse.iCode,
            sResponse: stResponse.sLastLine,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn stFailureAt(sCommand: &'static str, sFailureResponse: &'static str) -> AppError {
        let oListener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let iPort = oListener.local_addr().unwrap().port();
        let hServer = tokio::spawn(async move {
            let (oStream, _) = oListener.accept().await.unwrap();
            let (oReadHalf, mut oWriteHalf) = oStream.into_split();
            let mut oReader = BufReader::new(oReadHalf);
            oWriteHalf.write_all(b"220 test ESMTP\r\n").await.unwrap();
            loop {
                let mut sLine = String::new();
                oReader.read_line(&mut sLine).await.unwrap();
                let sLine = sLine.trim_end();
                if sLine.starts_with(sCommand) {
                    oWriteHalf
                        .write_all(sFailureResponse.as_bytes())
                        .await
                        .unwrap();
                    break;
                }
                let sResponse = if sLine == "DATA" {
                    "354 continue\r\n"
                } else {
                    "250 ok\r\n"
                };
                oWriteHalf.write_all(sResponse.as_bytes()).await.unwrap();
            }
        });

        let oSender = CSmtpEmailSender::new("127.0.0.1", iPort, "test-host");
        let stError = oSender
            .vSend(&StEmailMessage {
                sFrom: "no-reply@linux.org.ru".to_owned(),
                sTo: "user@example.org".to_owned(),
                sSubject: "Test".to_owned(),
                sBody: "body".to_owned(),
            })
            .await
            .expect_err("fake SMTP server must reject the selected command");
        hServer.await.unwrap();
        stError
    }

    #[test]
    fn wire_message_encodes_unicode_subject_and_dot_stuffs_body() {
        let sMessage = sWireMessage(&StEmailMessage {
            sFrom: "no-reply@linux.org.ru".to_string(),
            sTo: "user@example.org".to_string(),
            sSubject: "Регистрация".to_string(),
            sBody: "first\n.hidden\nlast".to_string(),
        })
        .unwrap();
        assert!(sMessage.contains("Subject: =?UTF-8?B?"));
        assert!(sMessage.contains("first\r\n..hidden\r\nlast"));
    }

    #[test]
    fn rejects_header_injection() {
        let stMessage = StEmailMessage {
            sFrom: "no-reply@linux.org.ru".to_string(),
            sTo: "user@example.org\r\nBcc: evil@example.org".to_string(),
            sSubject: "test".to_string(),
            sBody: String::new(),
        };
        assert!(sWireMessage(&stMessage).is_err());
    }

    #[test]
    fn rejects_smtp_envelope_delimiter_injection() {
        for sMailbox in [
            "user@example.org> SIZE=1",
            "user@example.org<evil@example.org",
            "user@@example.org",
        ] {
            assert!(stValidateMailbox(sMailbox).is_err(), "{sMailbox}");
        }
        assert_eq!(
            stValidateMailbox(" user@example.org").unwrap().sAddress,
            "user@example.org"
        );
    }

    #[tokio::test]
    async fn rejects_smtp_helo_command_injection_before_connecting() {
        let oSender = CSmtpEmailSender::new("127.0.0.1", 1, "localhost\r\nMAIL FROM:<evil>");
        let stError = oSender
            .vSend(&StEmailMessage {
                sFrom: "no-reply@linux.org.ru".to_owned(),
                sTo: "admin@example.org".to_owned(),
                sSubject: "test".to_owned(),
                sBody: "body".to_owned(),
            })
            .await
            .expect_err("invalid HELO must fail before SMTP connection");
        assert!(stError.to_string().contains("SMTP_HELO_NAME"));
    }

    #[tokio::test]
    async fn sends_a_complete_message_to_a_local_smtp_server() {
        let oListener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let iPort = oListener.local_addr().unwrap().port();
        let hServer = tokio::spawn(async move {
            let (oStream, _) = oListener.accept().await.unwrap();
            let (oReadHalf, mut oWriteHalf) = oStream.into_split();
            let mut oReader = BufReader::new(oReadHalf);
            oWriteHalf.write_all(b"220 test ESMTP\r\n").await.unwrap();

            for (sPrefix, sResponse) in [
                ("HELO test-host", "250 hello\r\n"),
                ("MAIL FROM:<no-reply@linux.org.ru>", "250 ok\r\n"),
                ("RCPT TO:<user@example.org>", "250 ok\r\n"),
                ("DATA", "354 continue\r\n"),
            ] {
                let mut sLine = String::new();
                oReader.read_line(&mut sLine).await.unwrap();
                assert_eq!(sLine.trim_end(), sPrefix);
                oWriteHalf.write_all(sResponse.as_bytes()).await.unwrap();
            }

            let mut sData = String::new();
            loop {
                let mut sLine = String::new();
                oReader.read_line(&mut sLine).await.unwrap();
                if sLine == ".\r\n" {
                    break;
                }
                sData.push_str(&sLine);
            }
            oWriteHalf.write_all(b"250 queued\r\n").await.unwrap();
            let mut sQuit = String::new();
            oReader.read_line(&mut sQuit).await.unwrap();
            assert_eq!(sQuit, "QUIT\r\n");
            oWriteHalf.write_all(b"221 bye\r\n").await.unwrap();
            sData
        });

        let oSender = CSmtpEmailSender::new("127.0.0.1", iPort, "test-host");
        oSender
            .vSend(&StEmailMessage {
                sFrom: "no-reply@linux.org.ru".to_string(),
                sTo: "user@example.org".to_string(),
                sSubject: "Test".to_string(),
                sBody: "hello\n.world".to_string(),
            })
            .await
            .unwrap();
        let sData = hServer.await.unwrap();
        assert!(sData.contains("Subject: Test\r\n"));
        assert!(sData.contains("Date: "));
        assert!(sData.contains("Message-ID: <"));
        assert!(sData.contains("hello\r\n..world\r\n"));
    }

    #[tokio::test]
    async fn only_rcpt_rejection_has_java_smtp_address_failed_classification() {
        let stRcptError = stFailureAt("RCPT TO:", "550 mailbox unavailable\r\n").await;
        assert!(matches!(
            stRcptError,
            AppError::SmtpAddressRejected { iStatus: 550, .. }
        ));

        for (sCommand, sResponse) in [
            ("HELO ", "550 bad HELO\r\n"),
            ("DATA", "554 transaction failed\r\n"),
        ] {
            let stError = stFailureAt(sCommand, sResponse).await;
            assert!(
                matches!(stError, AppError::Anyhow(_)),
                "{sCommand} failure must remain an infrastructure error"
            );
        }
    }
}
