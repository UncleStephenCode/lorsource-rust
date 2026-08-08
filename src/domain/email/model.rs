#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StEmailMessage {
    pub sFrom: String,
    pub sTo: String,
    pub sSubject: String,
    pub sBody: String,
}
