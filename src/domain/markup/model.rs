use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StMarkupUser {
    pub sInputNick: String,
    pub sCanonicalNick: String,
    pub bBlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StMarkupSource {
    pub sMessage: String,
    pub sMarkup: String,
}

/// Users resolved for one rendering batch.  A missing key deliberately means
/// that Java's `UserService.getUserCached` would throw `UserNotFoundException`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StMarkupUserDirectory {
    mapByInputNick: HashMap<String, StMarkupUser>,
}

impl StMarkupUserDirectory {
    pub fn stFromUsers(vecUsers: Vec<StMarkupUser>) -> Self {
        Self {
            mapByInputNick: vecUsers
                .into_iter()
                .map(|stUser| (stUser.sInputNick.clone(), stUser))
                .collect(),
        }
    }

    pub fn optFind(&self, sInputNick: &str) -> Option<&StMarkupUser> {
        self.mapByInputNick.get(sInputNick)
    }

    #[cfg(test)]
    pub fn iLen(&self) -> usize {
        self.mapByInputNick.len()
    }

    #[cfg(test)]
    pub fn bIsEmpty(&self) -> bool {
        self.mapByInputNick.is_empty()
    }
}
