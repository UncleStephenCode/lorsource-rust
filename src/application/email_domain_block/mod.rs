use chrono::{Months, Utc};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    domain::email_domain_block::{
        model::StEmailDomainBlockPage, repository::TrEmailDomainBlockRepository,
    },
    error::{AppError, Result},
    profile::ProfileSettings,
};

static ST_DOMAIN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$").expect("valid email domain regex")
});

#[derive(Debug, Clone)]
pub struct CEmailDomainBlockService<R>
where
    R: TrEmailDomainBlockRepository,
{
    oRepository: R,
}

impl<R> CEmailDomainBlockService<R>
where
    R: TrEmailDomainBlockRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn stListManual(
        &self,
        iModeratorId: i32,
        iOffset: i32,
    ) -> Result<StEmailDomainBlockPage> {
        let iCount = self.oRepository.iManualCount().await?;
        if iOffset < 0 || (iCount > 0 && i64::from(iOffset) >= iCount) {
            return Err(AppError::stBadInput("Wrong offset"));
        }

        let optSettings = self.oRepository.optProfileSettings(iModeratorId).await?;
        let iLimit = ProfileSettings::from_hstore_text(optSettings).messages;
        let vecBlocks = if iCount > 0 {
            self.oRepository.vecManualBlocks(iOffset, iLimit).await?
        } else {
            Vec::new()
        };

        Ok(StEmailDomainBlockPage {
            vecBlocks,
            iOffset,
            iLimit,
            bHasMore: iCount > i64::from(iOffset) + i64::from(iLimit),
        })
    }

    pub async fn vBlockManual(&self, sDomain: &str, iModeratorId: i32) -> Result<()> {
        let sDomain = Self::sNormalizeDomain(sDomain)?;
        let dtBlockUntil = Utc::now()
            .checked_add_months(Months::new(36))
            .ok_or_else(|| AppError::BadRequest("Invalid block expiry".to_string()))?;
        self.oRepository
            .vBlockManual(&sDomain, dtBlockUntil, iModeratorId)
            .await
    }

    pub async fn vUnblock(&self, sDomain: &str) -> Result<()> {
        // EmailDomainsBlockController.delete calls only `normalize`; the
        // add-only length/regex validation must not reject an existing legacy
        // domain selected for removal.
        let sDomain = Self::sNormalizeDomainForUnblock(sDomain)?;
        self.oRepository.vUnblock(&sDomain).await
    }

    pub async fn bIsBlocked(&self, sDomain: &str) -> Result<bool> {
        self.oRepository.bIsBlocked(&sDomain.to_lowercase()).await
    }

    pub fn sNormalizeDomain(sDomain: &str) -> Result<String> {
        let sNormalized = Self::sNormalizeDomainForUnblock(sDomain)?;
        if sNormalized.chars().count() > 255 || !ST_DOMAIN_RE.is_match(&sNormalized) {
            return Err(AppError::stBadInput("Invalid domain"));
        }
        Ok(sNormalized)
    }

    pub fn sNormalizeDomainForUnblock(sDomain: &str) -> Result<String> {
        let sNormalized = sDomain.trim().to_lowercase();
        if sNormalized.is_empty() {
            return Err(AppError::stBadInput("Empty domain"));
        }
        Ok(sNormalized)
    }
}

#[cfg(test)]
mod tests {
    use super::CEmailDomainBlockService;
    use crate::infra::postgres::email_domain_block_repository::CEmailDomainBlockPgRepository;

    type TyService = CEmailDomainBlockService<CEmailDomainBlockPgRepository>;

    #[test]
    fn normalizes_manual_domain_like_java_controller() {
        assert_eq!(
            TyService::sNormalizeDomain("  Mail.Example.COM ").expect("valid domain"),
            "mail.example.com"
        );
    }

    #[test]
    fn accepts_java_domain_regex_boundary_cases() {
        for sDomain in ["a", "a-b.example", "a.b", "0.example"] {
            assert!(TyService::sNormalizeDomain(sDomain).is_ok(), "{sDomain}");
        }
    }

    #[test]
    fn rejects_empty_invalid_and_too_long_domains() {
        for sDomain in ["", " ", ".example", "example.", "exam_ple.org", "пример.рф"] {
            assert!(TyService::sNormalizeDomain(sDomain).is_err(), "{sDomain}");
        }
        assert!(TyService::sNormalizeDomain(&"a".repeat(256)).is_err());
    }

    #[test]
    fn delete_only_normalizes_and_accepts_legacy_nonempty_domains() {
        assert_eq!(
            TyService::sNormalizeDomainForUnblock("  EXAM_PLE.ORG  ").unwrap(),
            "exam_ple.org"
        );
        assert_eq!(
            TyService::sNormalizeDomainForUnblock(&"A".repeat(256)).unwrap(),
            "a".repeat(256)
        );
        assert!(TyService::sNormalizeDomainForUnblock("  ").is_err());
    }
}
