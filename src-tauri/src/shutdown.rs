//! Exit requests require a frontend draft flush acknowledgement, never a timer.
#[derive(Default)]
pub struct Gate {
    pub registered: bool,
    pub allowed: bool,
    pending: Option<String>,
}
impl Gate {
    pub fn request(&mut self) -> Option<String> {
        if self.pending.is_some() {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.pending = Some(id.clone());
        Some(id)
    }
    pub fn finish(&mut self, id: &str, saved: bool, busy: bool) -> Result<bool, String> {
        if self.pending.as_deref() != Some(id) {
            return Err("INVALID_SHUTDOWN_REQUEST".into());
        }
        self.pending = None;
        self.allowed = saved && !busy;
        Ok(self.allowed)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_matching_ack_and_preserves_pending_on_forgery() {
        let mut g = Gate::default();
        let id = g.request().unwrap();
        assert!(g.request().is_none());
        assert!(g.finish("wrong", true, false).is_err());
        assert!(g.finish(&id, true, false).unwrap());
        assert!(g.allowed);
        assert!(g.finish(&id, true, false).is_err());
    }
    #[test]
    fn failure_and_active_operation_keep_app_open_and_allow_retry() {
        let mut g = Gate::default();
        for (saved, busy) in [(false, false), (true, true), (false, true)] {
            let id = g.request().unwrap();
            assert!(!g.finish(&id, saved, busy).unwrap());
            assert!(!g.allowed);
        }
        let id = g.request().unwrap();
        assert!(g.finish(&id, true, false).unwrap());
    }
}
