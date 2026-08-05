use std::time::Instant;

use super::App;
use super::types::FLASH_TTL;

#[derive(Debug, Default, Clone)]
pub struct Flash {
    current: Option<(String, Instant)>,
}

impl Flash {
    pub fn set(&mut self, msg: impl Into<String>) {
        self.current = Some((msg.into(), Instant::now()));
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    #[must_use] 
    pub fn active(&self) -> Option<&str> {
        self.current
            .as_ref()
            .filter(|(_, t)| t.elapsed() < FLASH_TTL)
            .map(|(m, _)| m.as_str())
    }

    #[must_use] 
    pub fn deadline(&self) -> Option<Instant> {
        self.current.as_ref().map(|(_, t)| *t + FLASH_TTL)
    }

    #[must_use] 
    pub fn should_clear(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() >= FLASH_TTL)
    }
}

impl App {
    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash_state.set(msg);
    }

    pub fn clear_flash(&mut self) {
        self.flash_state.clear();
    }

    pub fn flash_active(&self) -> Option<&str> {
        self.flash_state.active()
    }

    pub fn flash_deadline(&self) -> Option<Instant> {
        self.flash_state.deadline()
    }

    pub fn flash_should_clear(&self) -> bool {
        self.flash_state.should_clear()
    }
}
