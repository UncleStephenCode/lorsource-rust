use std::{collections::HashSet, time::Duration};

use tokio::sync::mpsc;

use crate::domain::{email::model::StEmailMessage, email::repository::TrEmailSender};

const I_MAX_MESSAGES: usize = 5;
const DT_RESET: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct StExceptionReport {
    pub sType: String,
    pub sBody: String,
}

#[derive(Debug, Clone, Default)]
pub struct CExceptionReporter {
    optSender: Option<mpsc::UnboundedSender<StExceptionReport>>,
}

impl CExceptionReporter {
    pub fn stNew<S>(optAdminEmail: Option<String>, oEmailSender: S) -> Self
    where
        S: TrEmailSender + 'static,
    {
        let Some(sAdminEmail) = optAdminEmail else {
            return Self::default();
        };
        // Pekko's default actor mailbox is unbounded. The five-minute actor
        // state, not mailbox overflow, decides what is sent or summarized.
        // Keeping every report also preserves the real high-rate count.
        let (oSender, oReceiver) = mpsc::unbounded_channel();
        tokio::spawn(vRunReporter(oReceiver, oEmailSender, sAdminEmail));
        Self {
            optSender: Some(oSender),
        }
    }

    pub fn vReport(&self, stReport: StExceptionReport) {
        let Some(oSender) = &self.optSender else {
            return;
        };
        if oSender.send(stReport).is_err() {
            tracing::error!("exception-report actor is closed");
        }
    }
}

#[derive(Debug, Default)]
struct StExceptionRateState {
    iCount: usize,
    setTypes: HashSet<String>,
}

impl StExceptionRateState {
    fn bShouldSend(&mut self, sType: &str) -> bool {
        self.iCount += 1;
        let bNewType = self.setTypes.insert(sType.to_owned());
        self.iCount < I_MAX_MESSAGES || bNewType
    }

    fn optResetSummary(&mut self) -> Option<(usize, String)> {
        let optSummary = (self.iCount >= I_MAX_MESSAGES).then(|| {
            let mut vecTypes = self.setTypes.iter().cloned().collect::<Vec<_>>();
            vecTypes.sort();
            (self.iCount, vecTypes.join("\n"))
        });
        self.iCount = 0;
        self.setTypes.clear();
        optSummary
    }
}

async fn vRunReporter<S>(
    mut oReceiver: mpsc::UnboundedReceiver<StExceptionReport>,
    oEmailSender: S,
    sAdminEmail: String,
) where
    S: TrEmailSender,
{
    let mut oReset = tokio::time::interval(DT_RESET);
    oReset.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    oReset.tick().await;
    let mut stRate = StExceptionRateState::default();
    loop {
        tokio::select! {
            optReport = oReceiver.recv() => {
                let Some(stReport) = optReport else { break };
                if stRate.bShouldSend(&stReport.sType) {
                    vSendReport(&oEmailSender, &sAdminEmail, &format!("Linux.org.ru: {}", stReport.sType), &stReport.sBody).await;
                } else {
                    tracing::warn!(error_type = %stReport.sType, "too many errors; skipped duplicate crash report");
                }
            }
            _ = oReset.tick() => {
                if let Some((iCount, sTypes)) = stRate.optResetSummary() {
                    vSendReport(
                        &oEmailSender,
                        &sAdminEmail,
                        &format!("Linux.org.ru: high exception rate ({iCount} in 5 minutes)"),
                        &sTypes,
                    ).await;
                }
            }
        }
    }
}

async fn vSendReport<S: TrEmailSender>(oSender: &S, sTo: &str, sSubject: &str, sBody: &str) {
    if let Err(stError) = oSender
        .vSend(&StEmailMessage {
            sFrom: "no-reply@linux.org.ru".to_owned(),
            sTo: sTo.to_owned(),
            sSubject: sSubject.to_owned(),
            sBody: sBody.to_owned(),
        })
        .await
    {
        tracing::error!(error = ?stError, "failed to send administrator crash report");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::{Duration, timeout};

    use crate::{error::Result, infra::smtp::CSmtpEmailSender};

    #[derive(Clone)]
    struct CRecordingSender {
        oSender: mpsc::UnboundedSender<StEmailMessage>,
    }

    #[async_trait]
    impl TrEmailSender for CRecordingSender {
        async fn vSend(&self, stMessage: &StEmailMessage) -> Result<()> {
            self.oSender
                .send(stMessage.clone())
                .expect("recording receiver");
            Ok(())
        }
    }

    #[test]
    fn rate_state_matches_java_first_four_and_new_type_rule() {
        let mut stRate = StExceptionRateState::default();
        assert!(stRate.bShouldSend("Sqlx"));
        assert!(stRate.bShouldSend("Sqlx"));
        assert!(stRate.bShouldSend("Sqlx"));
        assert!(stRate.bShouldSend("Sqlx"));
        assert!(!stRate.bShouldSend("Sqlx"));
        assert!(stRate.bShouldSend("Io"));
        assert_eq!(stRate.optResetSummary(), Some((6, "Io\nSqlx".to_owned())));
        assert!(stRate.bShouldSend("Sqlx"));
    }

    #[tokio::test]
    async fn reporter_mailbox_preserves_actor_order_and_rate_limit() {
        let (oEmailSender, mut oEmailReceiver) = mpsc::unbounded_channel();
        let cReporter = CExceptionReporter::stNew(
            Some("admin@example.org".to_owned()),
            CRecordingSender {
                oSender: oEmailSender,
            },
        );
        for iIndex in 0..100 {
            cReporter.vReport(StExceptionReport {
                sType: "Sqlx".to_owned(),
                sBody: format!("failure {iIndex}"),
            });
        }
        for sType in ["Io", "Done"] {
            cReporter.vReport(StExceptionReport {
                sType: sType.to_owned(),
                sBody: sType.to_owned(),
            });
        }

        let mut vecMessages = Vec::new();
        while vecMessages
            .last()
            .is_none_or(|stMessage: &StEmailMessage| stMessage.sSubject != "Linux.org.ru: Done")
        {
            vecMessages.push(
                timeout(Duration::from_secs(2), oEmailReceiver.recv())
                    .await
                    .expect("report delivery timeout")
                    .expect("reporter sender"),
            );
        }

        assert_eq!(vecMessages.len(), 6);
        assert_eq!(
            vecMessages
                .iter()
                .filter(|stMessage| stMessage.sSubject == "Linux.org.ru: Sqlx")
                .count(),
            4
        );
        assert_eq!(vecMessages[4].sSubject, "Linux.org.ru: Io");
        assert_eq!(vecMessages[5].sSubject, "Linux.org.ru: Done");
        assert!(vecMessages.iter().all(|stMessage| {
            stMessage.sFrom == "no-reply@linux.org.ru" && stMessage.sTo == "admin@example.org"
        }));
    }

    #[tokio::test]
    async fn reporter_delivers_complete_message_to_smtp_sink() {
        let oListener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("SMTP sink listener");
        let iPort = oListener.local_addr().expect("SMTP sink address").port();
        let hServer = tokio::spawn(async move {
            let (oStream, _) = oListener.accept().await.expect("SMTP connection");
            let (oReadHalf, mut oWriteHalf) = oStream.into_split();
            let mut oReader = BufReader::new(oReadHalf);
            oWriteHalf.write_all(b"220 test ESMTP\r\n").await.unwrap();

            for (sPrefix, sResponse) in [
                ("HELO ", "250 hello\r\n"),
                ("MAIL FROM:<no-reply@linux.org.ru>", "250 sender\r\n"),
                ("RCPT TO:<admin@example.org>", "250 recipient\r\n"),
                ("DATA", "354 continue\r\n"),
            ] {
                let mut sLine = String::new();
                oReader.read_line(&mut sLine).await.unwrap();
                assert!(
                    sLine.starts_with(sPrefix),
                    "unexpected SMTP line: {sLine:?}"
                );
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

        let cReporter = CExceptionReporter::stNew(
            Some("admin@example.org".to_owned()),
            CSmtpEmailSender::new("127.0.0.1", iPort, "test-host"),
        );
        cReporter.vReport(StExceptionReport {
            sType: "sqlx::Error".to_owned(),
            sBody: "POST: https://www.linux.org.ru/add.jsp\nIP: 192.0.2.1".to_owned(),
        });

        let sData = timeout(Duration::from_secs(3), hServer)
            .await
            .expect("SMTP report timeout")
            .expect("SMTP sink task");
        assert!(sData.contains("To: admin@example.org\r\n"));
        assert!(sData.contains("Subject: Linux.org.ru: sqlx::Error\r\n"));
        assert!(sData.contains("POST: https://www.linux.org.ru/add.jsp\r\n"));
        assert!(sData.contains("IP: 192.0.2.1"));
    }
}
