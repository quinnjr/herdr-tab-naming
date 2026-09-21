//! Test doubles and fixtures.

use crate::herdr::{Pane, Tab};
use crate::sync::Herdr;
use std::cell::RefCell;

pub fn pane(pane_id: &str, tab_id: &str, cwd: &str) -> Pane {
    let workspace_id = tab_id.split(':').next().unwrap_or("w").to_string();
    Pane {
        pane_id: pane_id.to_string(),
        tab_id: tab_id.to_string(),
        workspace_id,
        cwd: Some(cwd.to_string()),
        foreground_cwd: Some(cwd.to_string()),
        agent: Some("opencode".to_string()),
        agent_status: Some("working".to_string()),
        terminal_title: None,
        focused: true,
    }
}

pub fn tab(tab_id: &str, label: &str) -> Tab {
    let workspace_id = tab_id.split(':').next().unwrap_or("w").to_string();
    Tab {
        tab_id: tab_id.to_string(),
        workspace_id,
        number: 1,
        label: label.to_string(),
        focused: true,
        pane_count: 1,
    }
}

/// In-memory Herdr that records renames and reflects them back into its tabs.
pub struct FakeHerdr {
    pub panes: Vec<Pane>,
    pub tabs: RefCell<Vec<Tab>>,
    pub renames: RefCell<Vec<(String, String)>>,
    pub fail: bool,
}

impl FakeHerdr {
    pub fn new(panes: Vec<Pane>, tabs: Vec<Tab>) -> Self {
        Self {
            panes,
            tabs: RefCell::new(tabs),
            renames: RefCell::new(Vec::new()),
            fail: false,
        }
    }

    pub fn rename_count(&self) -> usize {
        self.renames.borrow().len()
    }
}

impl Herdr for FakeHerdr {
    fn pane_list(&self) -> Result<Vec<Pane>, String> {
        Ok(self.panes.clone())
    }

    fn tab_list(&self) -> Result<Vec<Tab>, String> {
        Ok(self.tabs.borrow().clone())
    }

    fn tab_rename(&self, tab_id: &str, label: &str) -> Result<(), String> {
        if self.fail {
            return Err("boom".to_string());
        }
        self.renames
            .borrow_mut()
            .push((tab_id.to_string(), label.to_string()));
        if let Some(tab) = self
            .tabs
            .borrow_mut()
            .iter_mut()
            .find(|t| t.tab_id == tab_id)
        {
            tab.label = label.to_string();
        }
        Ok(())
    }
}
