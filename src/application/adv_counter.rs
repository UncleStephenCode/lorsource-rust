use std::{collections::HashMap, sync::Mutex};

/// In-memory equivalent of Java's `AdvCounterActor` state. HTTP requests only
/// update this small map; the background worker persists one aggregated row
/// per path and minute instead of issuing a database query for every image.
#[derive(Default)]
pub struct CAdvCounter {
    mapCounters: Mutex<HashMap<String, i64>>,
}

impl CAdvCounter {
    pub fn vCount(&self, sPath: String) {
        let mut mapCounters = self
            .mapCounters
            .lock()
            .unwrap_or_else(|stPoisoned| stPoisoned.into_inner());
        let iCounter = mapCounters.entry(sPath).or_default();
        *iCounter = iCounter.saturating_add(1);
    }

    pub fn mapTake(&self) -> HashMap<String, i64> {
        let mut mapCounters = self
            .mapCounters
            .lock()
            .unwrap_or_else(|stPoisoned| stPoisoned.into_inner());
        std::mem::take(&mut *mapCounters)
    }

    /// A failed transaction must not discard already acknowledged requests.
    /// Merge its batch with requests which arrived while PostgreSQL was down.
    pub fn vRestore(&self, mapFailed: HashMap<String, i64>) {
        let mut mapCounters = self
            .mapCounters
            .lock()
            .unwrap_or_else(|stPoisoned| stPoisoned.into_inner());
        for (sPath, iIncrement) in mapFailed {
            let iCounter = mapCounters.entry(sPath).or_default();
            *iCounter = iCounter.saturating_add(iIncrement);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_paths_and_restores_failed_flush_without_losing_new_counts() {
        let cCounter = CAdvCounter::default();
        cCounter.vCount("/adv/320.png".to_owned());
        cCounter.vCount("/adv/320.png".to_owned());
        cCounter.vCount("/adv/728.png".to_owned());

        let mapFailed = cCounter.mapTake();
        assert_eq!(mapFailed.get("/adv/320.png"), Some(&2));
        assert_eq!(mapFailed.get("/adv/728.png"), Some(&1));
        assert!(cCounter.mapTake().is_empty());

        cCounter.vCount("/adv/320.png".to_owned());
        cCounter.vRestore(mapFailed);
        let mapRestored = cCounter.mapTake();
        assert_eq!(mapRestored.get("/adv/320.png"), Some(&3));
        assert_eq!(mapRestored.get("/adv/728.png"), Some(&1));
    }
}
