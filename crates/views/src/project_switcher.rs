//! Open/closed state for the sidebar project switcher.

/// The switcher is a menu, not a modal: one flag and a close on selection.
#[derive(Debug, Default)]
pub struct ProjectSwitcher {
    open: bool,
}

impl ProjectSwitcher {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_closes() {
        let mut switcher = ProjectSwitcher::default();
        assert!(!switcher.is_open());
        switcher.toggle();
        assert!(switcher.is_open());
        switcher.close();
        assert!(!switcher.is_open());
        switcher.open();
        assert!(switcher.is_open());
    }
}
