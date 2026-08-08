use crate::{
    domain::{email::model::StEmailMessage, email::repository::TrEmailSender},
    error::Result,
    security::secret_tokens,
};

const S_FROM: &str = "no-reply@linux.org.ru";

#[derive(Debug, Clone)]
pub struct CEmailService<S>
where
    S: TrEmailSender,
{
    oSender: S,
    sSiteSecret: String,
}

impl<S> CEmailService<S>
where
    S: TrEmailSender,
{
    pub fn new(oSender: S, sSiteSecret: impl Into<String>) -> Self {
        Self {
            oSender,
            sSiteSecret: sSiteSecret.into(),
        }
    }

    pub async fn vSendRegistration(
        &self,
        sNick: &str,
        sEmail: &str,
        iRegistrationMillis: i64,
        bNew: bool,
    ) -> Result<()> {
        let sCode =
            secret_tokens::activation_code(&self.sSiteSecret, sNick, sEmail, iRegistrationMillis);
        let sBody = sRegistrationBody(sNick, &sCode, bNew);
        self.oSender
            .vSend(&StEmailMessage {
                sFrom: S_FROM.to_string(),
                sTo: sEmail.to_string(),
                sSubject: "Регистрация на Linux.org.ru".to_string(),
                sBody,
            })
            .await
    }

    pub async fn vSendPasswordReset(
        &self,
        sNick: &str,
        sEmail: &str,
        sResetCode: &str,
    ) -> Result<()> {
        let sBody = format!(
            "Здравствуйте!\n\n\
             Для сброса вашего пароля перейдите по ссылке https://www.linux.org.ru/reset-password\n\n\
             Ваш ник {sNick}, код подтверждения: {sResetCode}\n\n\
             Если это были не вы, то просто игнорируйте это письмо.\n\n\
             Удачи!"
        );
        self.oSender
            .vSend(&StEmailMessage {
                sFrom: S_FROM.to_string(),
                sTo: sEmail.to_string(),
                sSubject: "Your password @linux.org.ru".to_string(),
                sBody,
            })
            .await
    }
}

fn sRegistrationBody(sNick: &str, sCode: &str, bNew: bool) -> String {
    let sRecordAction = if bNew {
        "появилась регистрационная запись"
    } else {
        "была изменена регистрационная запись"
    };
    let sConfirmation = if bNew {
        "Если же именно вы решили зарегистрироваться на форуме по адресу https://www.linux.org.ru/,\n\
         то вам следует подтвердить свою регистрацию и тем самым активировать вашу учетную запись."
    } else {
        "Если же именно вы решили изменить свою регистрационную запись https://www.linux.org.ru/,\n\
         то вам следует подтвердить свое изменение."
    };
    let sEncodedCode = urlencoding::encode(sCode);
    format!(
        "Здравствуйте!\n\n\
         На форуме по адресу https://www.linux.org.ru/ {sRecordAction},\n\
         в которой был указан ваш электронный адрес (e-mail).\n\n\
         При заполнении регистрационной формы было указано следующее имя пользователя: '{sNick}'\n\n\
         Если вы не понимаете, о чем идет речь - просто проигнорируйте это сообщение!\n\n\
         {sConfirmation}\n\n\
         Для активации перейдите по ссылке:\n\n\
         https://www.linux.org.ru/activate?nick={sNick}&activation={sEncodedCode}\n\n\
         (код активации: {sCode})\n\n\
         Благодарим за регистрацию!\n"
    )
}

#[cfg(test)]
mod tests {
    use super::sRegistrationBody;

    #[test]
    fn registration_message_contains_java_activation_contract() {
        let sBody = sRegistrationBody("test-user", "a+b/c=", true);
        assert!(sBody.contains("https://www.linux.org.ru/activate?nick=test-user"));
        assert!(sBody.contains("activation=a%2Bb%2Fc%3D"));
        assert!(sBody.contains("(код активации: a+b/c=)"));
    }

    #[test]
    fn changed_registration_uses_the_original_wording() {
        let sBody = sRegistrationBody("test", "code", false);
        assert!(sBody.contains("была изменена регистрационная запись"));
        assert!(sBody.contains("подтвердить свое изменение"));
    }
}
