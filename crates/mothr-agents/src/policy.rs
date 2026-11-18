//! Mothr's unified permission policy, applied on top of whatever the agent supports.

use mothr_core::{ModeInfo, PermissionMode, PermissionOptionKind, ToolKind};

/// What to do with a permission request under a mode, without asking the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDecision {
    Allow,
    Reject,
    Ask,
}

pub fn decide(mode: PermissionMode, kind: ToolKind) -> AutoDecision {
    use AutoDecision::*;
    match mode {
        PermissionMode::AllowAll => Allow,
        PermissionMode::Ask => Ask,
        PermissionMode::AcceptEdits => match kind {
            ToolKind::Edit | ToolKind::Read | ToolKind::Search | ToolKind::Think => Allow,
            _ => Ask,
        },
        PermissionMode::Auto => match kind {
            ToolKind::Read | ToolKind::Search | ToolKind::Think | ToolKind::Fetch | ToolKind::Edit => Allow,
            _ => Ask,
        },
        PermissionMode::Plan => match kind {
            ToolKind::Read | ToolKind::Search | ToolKind::Think | ToolKind::Fetch => Allow,
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move | ToolKind::Execute => Reject,
            _ => Ask,
        },
    }
}

/// Pick the option id matching a decision from an agent-provided option list.
pub fn pick_option(
    decision: AutoDecision,
    options: &[(String, PermissionOptionKind)],
) -> Option<&str> {
    let prefs: &[PermissionOptionKind] = match decision {
        AutoDecision::Allow => &[PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways],
        AutoDecision::Reject => &[PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways],
        AutoDecision::Ask => return None,
    };
    prefs
        .iter()
        .find_map(|p| options.iter().find(|(_, k)| k == p).map(|(id, _)| id.as_str()))
}

fn norm(id: &str) -> String {
    let tail = id.rsplit(['#', '/']).next().unwrap_or(id);
    tail.to_ascii_lowercase().replace(['-', '_', ' '], "")
}

/// Map a Mothr mode onto the closest agent-advertised ACP mode id.
pub fn match_mode(mode: PermissionMode, modes: &[ModeInfo]) -> Option<String> {
    let candidates: &[&str] = match mode {
        PermissionMode::Ask => &["default", "ask", "agent", "build", "normal", "code", "chat"],
        PermissionMode::AcceptEdits => &["acceptedits", "autoedit", "edit", "dontask"],
        PermissionMode::Auto => &["auto", "agent", "default", "build"],
        PermissionMode::Plan => &["plan", "architect", "readonly", "ask"],
        PermissionMode::AllowAll => &["bypasspermissions", "yolo", "fullaccess", "allowall", "autopilot", "dontask"],
    };
    candidates
        .iter()
        .find_map(|c| modes.iter().find(|m| norm(&m.id) == *c || norm(&m.name) == *c))
        .map(|m| m.id.clone())
}

/// Reverse mapping: agent mode id → closest Mothr mode.
pub fn mode_from_agent(id: &str) -> Option<PermissionMode> {
    Some(match norm(id).as_str() {
        "plan" | "architect" | "readonly" => PermissionMode::Plan,
        "acceptedits" | "autoedit" => PermissionMode::AcceptEdits,
        "bypasspermissions" | "yolo" | "fullaccess" | "allowall" | "autopilot" => PermissionMode::AllowAll,
        "auto" => PermissionMode::Auto,
        "default" | "ask" | "agent" | "build" | "normal" | "code" => PermissionMode::Ask,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(ids: &[&str]) -> Vec<ModeInfo> {
        ids.iter()
            .map(|i| ModeInfo { id: i.to_string(), name: i.to_string(), description: None })
            .collect()
    }

    #[test]
    fn copilot_modes() {
        let m = modes(&[
            "https://agentclientprotocol.com/protocol/session-modes#agent",
            "https://agentclientprotocol.com/protocol/session-modes#plan",
            "https://agentclientprotocol.com/protocol/session-modes#autopilot",
        ]);
        assert!(match_mode(PermissionMode::Plan, &m).unwrap().ends_with("#plan"));
        assert!(match_mode(PermissionMode::AllowAll, &m).unwrap().ends_with("#autopilot"));
        assert!(match_mode(PermissionMode::Ask, &m).unwrap().ends_with("#agent"));
        assert_eq!(mode_from_agent(&m[2].id), Some(PermissionMode::AllowAll));
    }

    #[test]
    fn hermes_modes() {
        let m = modes(&["default", "accept_edits", "dont_ask"]);
        assert_eq!(match_mode(PermissionMode::AcceptEdits, &m).as_deref(), Some("accept_edits"));
        assert_eq!(match_mode(PermissionMode::AllowAll, &m).as_deref(), Some("dont_ask"));
        assert_eq!(match_mode(PermissionMode::Plan, &m), None);
    }

    #[test]
    fn plan_rejects_edits() {
        assert_eq!(decide(PermissionMode::Plan, ToolKind::Edit), AutoDecision::Reject);
        assert_eq!(decide(PermissionMode::AllowAll, ToolKind::Execute), AutoDecision::Allow);
        let opts = vec![("a".to_string(), PermissionOptionKind::AllowAlways), ("r".to_string(), PermissionOptionKind::RejectOnce)];
        assert_eq!(pick_option(AutoDecision::Allow, &opts), Some("a"));
        assert_eq!(pick_option(AutoDecision::Reject, &opts), Some("r"));
    }
}
