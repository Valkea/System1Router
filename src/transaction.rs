use crate::protocol::{Diagnostic, ModelState};
use std::{collections::HashMap, sync::{Arc, Mutex}};

pub trait OverrideAdapter {
    fn snapshot(&self) -> Result<ModelState, String>;
    fn apply(&mut self, state: &ModelState) -> Result<(), String>;
    fn restore(&mut self, state: &ModelState) -> Result<(), String>;
    fn verify(&self, state: &ModelState) -> bool { self.snapshot().map(|current| current == *state).unwrap_or(false) }
}

#[derive(Debug, Clone)]
pub struct RestorationFailure {
    pub diagnostic: Diagnostic,
    pub retries: u8,
}

#[derive(Debug, Clone, Default)]
pub struct OverrideCoordinator { locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>> }

impl OverrideCoordinator {
    pub fn new() -> Self { Self::default() }
    fn lock_for(&self, scope: &str) -> Arc<Mutex<()>> { let mut locks = self.locks.lock().expect("override lock poisoned"); locks.entry(scope.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone() }

    pub fn execute<A, F, R>(&self, adapter: &mut A, target: &ModelState, retries: u8, run: F) -> Result<(R, Option<RestorationFailure>), String>
    where A: OverrideAdapter, F: FnOnce() -> Result<R, String> {
        let scope = target.scope.clone().unwrap_or_else(|| "default".into());
        let lock = self.lock_for(&scope);
        let _guard = lock.lock().map_err(|_| "override lock poisoned".to_string())?;
        let original = adapter.snapshot()?;
        if let Err(error) = adapter.apply(target) { return Err(format!("apply override: {error}")); }
        let request_result = run();
        let mut restored = false;
        let mut last_error = String::new();
        for _ in 0..=retries {
            match adapter.restore(&original) {
                Ok(()) if adapter.verify(&original) => { restored = true; break; }
                Ok(()) => last_error = "restoration verification failed".into(),
                Err(error) => last_error = error,
            }
        }
        if restored { return request_result.map(|value| (value, None)); }
        let failure = RestorationFailure { diagnostic: Diagnostic::error("restoration_failed", format!("model state may require manual correction: {last_error}")), retries };
        request_result.map(|value| (value, Some(failure))).or_else(|error| Err(format!("{error}; restoration failed: {last_error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake { state: ModelState, fail_restore: bool }
    impl OverrideAdapter for Fake {
        fn snapshot(&self) -> Result<ModelState, String> { Ok(self.state.clone()) }
        fn apply(&mut self, state: &ModelState) -> Result<(), String> { self.state = state.clone(); Ok(()) }
        fn restore(&mut self, state: &ModelState) -> Result<(), String> { if self.fail_restore { return Err("no".into()); } self.state = state.clone(); Ok(()) }
    }
    #[test] fn restores_after_request_failure() { let original = ModelState { provider: "p".into(), model: "old".into(), variant: None, scope: Some("s".into()) }; let target = ModelState { provider: "p".into(), model: "new".into(), variant: None, scope: Some("s".into()) }; let mut fake = Fake { state: original.clone(), fail_restore: false }; let result = OverrideCoordinator::new().execute(&mut fake, &target, 1, || Err::<(), _>("request failed".into())); assert!(result.is_err()); assert_eq!(fake.state, original); }
}
