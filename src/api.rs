//! serde types for the herdr socket API (protocol 22), written from `herdr api schema --json`.
//! Only the fields this plugin needs; unknown fields are ignored.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SOURCE: &str = "plugin:vndv.project-tiles";
pub const PLUGIN_ID: &str = "vndv.project-tiles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub label: String,
    pub number: u64,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub active_tab_id: Option<String>,
    pub tokens: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub number: u64,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub agent: Option<String>,
    pub display_agent: Option<String>,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub label: Option<String>,
}

impl PaneInfo {
    pub fn cwds(&self) -> impl Iterator<Item = &str> {
        self.cwd
            .as_deref()
            .into_iter()
            .chain(self.foreground_cwd.as_deref())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SessionSnapshot {
    pub version: String,
    pub protocol: u64,
    pub focused_workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
}

/// `HERDR_PLUGIN_CONTEXT_JSON`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct InvocationContext {
    pub workspace_id: Option<String>,
    pub workspace_label: Option<String>,
    pub workspace_cwd: Option<String>,
    pub tab_id: Option<String>,
    pub tab_label: Option<String>,
    pub focused_pane_id: Option<String>,
    pub focused_pane_cwd: Option<String>,
    pub focused_pane_agent: Option<String>,
    pub focused_pane_status: Option<AgentStatus>,
    pub invocation_source: Option<String>,
}

impl InvocationContext {
    pub fn from_env() -> InvocationContext {
        std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    /// The folder the user is in: focused pane cwd, else workspace cwd.
    pub fn here(&self) -> Option<String> {
        self.focused_pane_cwd
            .clone()
            .or_else(|| self.workspace_cwd.clone())
            .filter(|s| !s.is_empty())
    }
}

/// A pushed event line: `{"event": "pane_created", "data": {"type": "pane_created", ...}}`.
#[derive(Debug, Clone, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    /// Payload; events are only invalidation signals here, kept for tests and debugging.
    #[serde(default)]
    #[allow(dead_code)]
    pub data: Value,
}

impl EventEnvelope {
    /// Event kind with dots: `pane.created`, `pane.agent_status_changed`.
    pub fn kind(&self) -> String {
        normalize_kind(&self.event)
    }
}

/// `pane_agent_status_changed` → `pane.agent_status_changed` (the first `_` is the group separator).
pub fn normalize_kind(k: &str) -> String {
    if k.contains('.') {
        return k.to_string();
    }
    match k.split_once('_') {
        Some((group, rest)) => format!("{group}.{rest}"),
        None => k.to_string(),
    }
}

/// Lifecycle subscriptions (no pane filter needed).
pub const LIFECYCLE: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.moved",
    "pane.created",
    "pane.updated",
    "pane.closed",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
];

pub fn lifecycle_subscriptions() -> Vec<Value> {
    LIFECYCLE
        .iter()
        .map(|t| serde_json::json!({ "type": t }))
        .collect()
}

/// `pane.agent_status_changed` requires a pane id, one entry per pane.
pub fn agent_subscriptions(pane_ids: &[String]) -> Vec<Value> {
    pane_ids
        .iter()
        .map(|p| serde_json::json!({ "type": "pane.agent_status_changed", "pane_id": p }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_from_schema_shape() {
        let c: InvocationContext = serde_json::from_str(
            r#"{"workspace_id":"w1","workspace_label":"api","workspace_cwd":"/repo","tab_id":"w1:t1",
                "focused_pane_id":"w1:p1","focused_pane_cwd":"/repo/sub","focused_pane_status":"working",
                "worktree":null,"invocation_source":"keybinding","new_field":1}"#,
        )
        .unwrap();
        assert_eq!(c.here().as_deref(), Some("/repo/sub"));
        assert_eq!(c.focused_pane_status, Some(AgentStatus::Working));
        let e: InvocationContext = serde_json::from_str("{}").unwrap();
        assert_eq!(e.here(), None);
    }

    #[test]
    fn snapshot_and_events() {
        let s: SessionSnapshot = serde_json::from_str(
            r#"{"version":"0.9.5","protocol":22,"focused_workspace_id":"w1","agents":[],"layouts":[],
                "workspaces":[{"workspace_id":"w1","label":"api","number":1,"focused":true,"active_tab_id":"w1:t1",
                  "agent_status":"idle","pane_count":1,"tab_count":1,"tokens":{"pt_id":"x"}}],
                "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"main","number":1,"focused":true,"agent_status":"idle","pane_count":1}],
                "panes":[{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","terminal_id":"t","focused":true,
                  "agent_status":"blocked","revision":3,"cwd":"/repo","foreground_cwd":null,"agent":"claude"}]}"#,
        )
        .unwrap();
        assert_eq!(s.workspaces[0].tokens["pt_id"], "x");
        assert_eq!(s.panes[0].agent_status, AgentStatus::Blocked);
        assert_eq!(s.panes[0].cwds().collect::<Vec<_>>(), vec!["/repo"]);
        let st: AgentStatus = serde_json::from_str(r#""future_state""#).unwrap();
        assert_eq!(st, AgentStatus::Unknown);

        let e: EventEnvelope = serde_json::from_str(
            r#"{"event":"pane_agent_status_changed","data":{"type":"pane_agent_status_changed","pane_id":"w1:p1","workspace_id":"w1","agent_status":"done"}}"#,
        )
        .unwrap();
        assert_eq!(e.kind(), "pane.agent_status_changed");
        assert_eq!(normalize_kind("workspace.created"), "workspace.created");
        assert_eq!(
            normalize_kind("workspace_metadata_updated"),
            "workspace.metadata_updated"
        );
        assert_eq!(
            agent_subscriptions(&["w1:p1".into()])[0]["pane_id"],
            "w1:p1"
        );
    }
}
