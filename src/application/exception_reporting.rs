use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::mpsc;

use crate::domain::{email::model::StEmailMessage, email::repository::TrEmailSender};

const I_MAX_MESSAGES: usize = 5;
const I_REPORT_QUEUE_CAPACITY: usize = 128;
const DT_RESET: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct StExceptionReport {
    pub sType: String,
    pub sBody: String,
}

#[derive(Debug, Clone, Default)]
pub struct CExceptionReporter {
    optSender: Option<mpsc::Sender<StExceptionReport>>,
    stMetrics: Arc<StExceptionReporterMetrics>,
    iQueueCapacity: usize,
}

#[derive(Debug, Default)]
struct StExceptionReporterMetrics {
    iDroppedPending: AtomicUsize,
    iDroppedTotal: AtomicUsize,
}

impl CExceptionReporter {
    pub fn stNew<S>(optAdminEmail: Option<String>, oEmailSender: S) -> Self
    where
        S: TrEmailSender + 'static,
    {
        Self::stNewWithOptions(
            optAdminEmail,
            oEmailSender,
            I_REPORT_QUEUE_CAPACITY,
            DT_RESET,
        )
    }

    fn stNewWithOptions<S>(
        optAdminEmail: Option<String>,
        oEmailSender: S,
        iQueueCapacity: usize,
        dtReset: Duration,
    ) -> Self
    where
        S: TrEmailSender + 'static,
    {
        let Some(sAdminEmail) = optAdminEmail else {
            return Self::default();
        };
        let iQueueCapacity = iQueueCapacity.max(1);
        let stMetrics = Arc::new(StExceptionReporterMetrics::default());
        let (oSender, oReceiver) = mpsc::channel(iQueueCapacity);
        tokio::spawn(vRunReporter(
            oReceiver,
            oEmailSender,
            sAdminEmail,
            Arc::clone(&stMetrics),
            dtReset,
        ));
        Self {
            optSender: Some(oSender),
            stMetrics,
            iQueueCapacity,
        }
    }

    pub fn vReport(&self, stReport: StExceptionReport) {
        let Some(oSender) = &self.optSender else {
            return;
        };
        match oSender.try_send(stReport) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                vSaturatingIncrement(&self.stMetrics.iDroppedPending);
                let iDropped = vSaturatingIncrement(&self.stMetrics.iDroppedTotal);
                if iDropped == 1 || iDropped.is_power_of_two() {
                    tracing::warn!(
                        dropped_reports = iDropped,
                        queue_capacity = self.iQueueCapacity,
                        "exception-report queue is full; reports are being aggregated"
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::error!("exception-report actor is closed");
            }
        }
    }

    #[cfg(test)]
    fn iDroppedReports(&self) -> usize {
        self.stMetrics.iDroppedTotal.load(Ordering::Relaxed)
    }
}

fn vSaturatingIncrement(oValue: &AtomicUsize) -> usize {
    let iPrevious = oValue
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |iValue| {
            Some(iValue.saturating_add(1))
        })
        .unwrap_or_else(|iValue| iValue);
    iPrevious.saturating_add(1)
}

#[derive(Debug, Default)]
struct StExceptionRateState {
    iCount: usize,
    iDropped: usize,
    setTypes: HashSet<String>,
}

impl StExceptionRateState {
    fn bShouldSend(&mut self, sType: &str) -> bool {
        self.iCount = self.iCount.saturating_add(1);
        let bNewType = self.setTypes.insert(sType.to_owned());
        self.iCount < I_MAX_MESSAGES || bNewType
    }

    fn vRecordDropped(&mut self, iDropped: usize) {
        self.iCount = self.iCount.saturating_add(iDropped);
        self.iDropped = self.iDropped.saturating_add(iDropped);
    }

    fn optResetSummary(&mut self) -> Option<(usize, String)> {
        let optSummary = (self.iCount >= I_MAX_MESSAGES).then(|| {
            let mut vecTypes = self.setTypes.iter().cloned().collect::<Vec<_>>();
            vecTypes.sort();
            let mut sBody = vecTypes.join("\n");
            if self.iDropped > 0 {
                if !sBody.is_empty() {
                    sBody.push('\n');
                }
                sBody.push_str(&format!(
                    "[{} crash reports dropped because the bounded reporter queue was full]",
                    self.iDropped
                ));
            }
            (self.iCount, sBody)
        });
        self.iCount = 0;
        self.iDropped = 0;
        self.setTypes.clear();
        optSummary
    }
}

async fn vRunReporter<S>(
    mut oReceiver: mpsc::Receiver<StExceptionReport>,
    oEmailSender: S,
    sAdminEmail: String,
    stMetrics: Arc<StExceptionReporterMetrics>,
    dtReset: Duration,
) where
    S: TrEmailSender,
{
    let mut oReset = tokio::time::interval(dtReset);
    oReset.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    oReset.tick().await;
    let mut stRate = StExceptionRateState::default();
    loop {
        // A full queue means every accepted report precedes the aggregated
        // drops. Drain those accepted reports in FIFO order before folding the
        // dropped count into the five-minute Java-compatible rate window.
        if oReceiver.is_empty() {
            stRate.vRecordDropped(stMetrics.iDroppedPending.swap(0, Ordering::Relaxed));
        }
        tokio::select! {
            biased;
            optReport = oReceiver.recv() => {
                let Some(stReport) = optReport else {
                    stRate.vRecordDropped(stMetrics.iDroppedPending.swap(0, Ordering::Relaxed));
                    break;
                };
                if stRate.bShouldSend(&stReport.sType) {
                    vSendReport(&oEmailSender, &sAdminEmail, &format!("Linux.org.ru: {}", stReport.sType), &stReport.sBody).await;
                } else {
                    tracing::warn!(error_type = %stReport.sType, "too many errors; skipped duplicate crash report");
                }
            }
            _ = oReset.tick() => {
                stRate.vRecordDropped(stMetrics.iDroppedPending.swap(0, Ordering::Relaxed));
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
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::Semaphore;
    use tokio::time::{Duration, timeout};

    use crate::{
        error::{AppError, Result},
        infra::smtp::CSmtpEmailSender,
    };

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

    #[derive(Clone)]
    struct CBlockingOutageSender {
        oStarted: mpsc::UnboundedSender<StEmailMessage>,
        oRelease: Arc<Semaphore>,
    }

    #[async_trait]
    impl TrEmailSender for CBlockingOutageSender {
        async fn vSend(&self, stMessage: &StEmailMessage) -> Result<()> {
            self.oStarted
                .send(stMessage.clone())
                .expect("outage observer");
            let _stPermit = self.oRelease.acquire().await.expect("outage release");
            Err(AppError::Anyhow(anyhow::anyhow!("SMTP unavailable")))
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
    async fn reporter_bounds_and_aggregates_during_smtp_outage() {
        let (oStartedSender, mut oStartedReceiver) = mpsc::unbounded_channel();
        let oRelease = Arc::new(Semaphore::new(0));
        let cReporter = CExceptionReporter::stNewWithOptions(
            Some("admin@example.org".to_owned()),
            CBlockingOutageSender {
                oStarted: oStartedSender,
                oRelease: Arc::clone(&oRelease),
            },
            2,
            Duration::from_millis(20),
        );

        cReporter.vReport(StExceptionReport {
            sType: "Sqlx".to_owned(),
            sBody: "failure 0".to_owned(),
        });
        let stFirst = timeout(Duration::from_secs(1), oStartedReceiver.recv())
            .await
            .expect("first send must start")
            .expect("outage observer");
        assert_eq!(stFirst.sBody, "failure 0");

        let dtStarted = std::time::Instant::now();
        for iIndex in 1..100 {
            cReporter.vReport(StExceptionReport {
                sType: "Sqlx".to_owned(),
                sBody: format!("failure {iIndex}"),
            });
        }
        assert!(
            dtStarted.elapsed() < Duration::from_millis(100),
            "reporting must remain nonblocking while SMTP is unavailable"
        );
        assert_eq!(cReporter.iDroppedReports(), 97);

        tokio::time::sleep(Duration::from_millis(30)).await;
        oRelease.add_permits(1);

        let mut vecAttempted = vec![stFirst];
        while vecAttempted
            .last()
            .is_none_or(|stMessage| !stMessage.sSubject.contains("high exception rate"))
        {
            vecAttempted.push(
                timeout(Duration::from_secs(1), oStartedReceiver.recv())
                    .await
                    .expect("aggregated report timeout")
                    .expect("outage observer"),
            );
        }

        assert_eq!(vecAttempted[1].sBody, "failure 1");
        assert_eq!(vecAttempted[2].sBody, "failure 2");
        assert_eq!(
            vecAttempted[3].sSubject,
            "Linux.org.ru: high exception rate (100 in 5 minutes)"
        );
        assert!(vecAttempted[3].sBody.contains("Sqlx"));
        assert!(vecAttempted[3].sBody.contains("97 crash reports dropped"));
        assert!(
            vecAttempted
                .iter()
                .all(|stMessage| !stMessage.sBody.contains("failure 3"))
        );
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
