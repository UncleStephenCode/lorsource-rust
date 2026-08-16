use crate::{
    domain::{
        email::address::optCanonicalInternetAddress,
        user::identity::{
            StActivationIdentity, StExactUserIdentity, StPasswordResetIdentity,
            StPasswordResetRequestIdentity, TrUserIdentityRepository,
        },
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CUserIdentityService<R>
where
    R: TrUserIdentityRepository,
{
    oRepository: R,
}

impl<R> CUserIdentityService<R>
where
    R: TrUserIdentityRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn optExactIdentity(&self, sNick: &str) -> Result<Option<StExactUserIdentity>> {
        self.oRepository.optExactIdentity(sNick).await
    }

    pub async fn optActivationIdentity(&self, sNick: &str) -> Result<Option<StActivationIdentity>> {
        self.oRepository.optActivationIdentity(sNick).await
    }

    pub async fn optPasswordResetIdentity(
        &self,
        sNick: &str,
    ) -> Result<Option<StPasswordResetIdentity>> {
        self.oRepository.optPasswordResetIdentity(sNick).await
    }

    /// Java `UserDao.getByEmail`: parse exactly one strict Internet mailbox,
    /// pass `InternetAddress.getAddress.toLowerCase` to PostgreSQL, then let
    /// the repository apply `normalize_email` and the legacy row ordering.
    pub async fn optPasswordResetRequestIdentity(
        &self,
        sSubmittedEmail: &str,
    ) -> Result<Option<StPasswordResetRequestIdentity>> {
        let Some(sAddress) = optPasswordResetLookupAddress(sSubmittedEmail) else {
            return Ok(None);
        };
        self.oRepository
            .optPasswordResetRequestIdentity(&sAddress)
            .await
    }

    pub async fn bExistsOrSimilar(&self, sNick: &str) -> Result<bool> {
        if self.oRepository.bExactNickExists(sNick).await? {
            return Ok(true);
        }
        self.oRepository.bSimilarNickExists(sNick).await
    }

    pub async fn optProfileSettings(&self, iUserId: i32) -> Result<Option<String>> {
        self.oRepository.optProfileSettings(iUserId).await
    }

    pub async fn vecEventTypes(&self, iUserId: i32) -> Result<Vec<String>> {
        self.oRepository.vecEventTypes(iUserId).await
    }
}

fn optPasswordResetLookupAddress(sSubmittedEmail: &str) -> Option<String> {
    optCanonicalInternetAddress(sSubmittedEmail)
}

#[cfg(test)]
mod tests {
    use super::optPasswordResetLookupAddress;

    #[test]
    fn password_reset_lookup_uses_strict_get_address_and_lowercase_semantics() {
        assert_eq!(
            optPasswordResetLookupAddress("  Example User <User.Name@GMAIL.COM>  ").as_deref(),
            Some("user.name@gmail.com")
        );
        assert!(optPasswordResetLookupAddress("first@example.org, second@example.org").is_none());
        assert!(
            optPasswordResetLookupAddress("user@example.org\r\nBcc:evil@example.org").is_none()
        );
    }
}
