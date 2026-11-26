//! The skill library: installing, removing and toggling skills, and handing the library to
//! every running agent when it changes.

use super::*;
use tokio::sync::oneshot;


impl Hub {
    /// Fetch skills in the background; the library and a toast follow when it's done.
    pub(crate) fn install_skills(&self, source: SkillSource) -> anyhow::Result<()> {
        {
            let mut st = self.state.write();
            anyhow::ensure!(!st.skills_installing.contains(&source), "{} is already being added", source.label());
            st.skills_installing.push(source.clone());
        }
        self.broadcast_skills();
        let Some(hub) = self.me.upgrade() else { return Ok(()) };
        self.rt.spawn(async move {
            let result = hub.skills.install(&source).await;
            hub.state.write().skills_installing.retain(|s| s != &source);
            match result {
                Ok(done) => {
                    hub.reload_skills();
                    hub.broadcast(HubEvent::Toast { level: NoticeLevel::Info, text: done.summary(&source) });
                }
                Err(e) => {
                    hub.broadcast_skills();
                    hub.broadcast(HubEvent::Toast { level: NoticeLevel::Error, text: format!("Couldn't add {}: {e}", source.label()) });
                }
            }
        });
        Ok(())
    }

    /// Re-read the library and pass it on: to views, to running agents (so `/name` expands
    /// with it), and into each running chat's `/` menu.
    pub(crate) fn reload_skills(&self) {
        let skills = self.skills.load();
        let menus: Vec<(SessionId, Vec<CommandInfo>)> = {
            let mut st = self.state.write();
            st.skills = skills.clone();
            for tx in st.drivers.values() {
                let _ = tx.send(SessionCommand::SetSkills { skills: skills.clone() });
            }
            st.sessions
                .iter()
                .filter(|s| st.drivers.contains_key(&s.meta.id))
                .filter_map(|s| {
                    let merged = merge_skill_commands(&s.commands, &skills);
                    (merged != s.commands).then_some((s.meta.id, merged))
                })
                .collect()
        };
        for (id, commands) in menus {
            self.on_session_event(id, SessionEvent::Commands { commands });
        }
        self.broadcast_skills();
    }

    fn broadcast_skills(&self) {
        let st = self.state.read();
        self.broadcast(HubEvent::Skills { skills: st.skills.clone(), installing: st.skills_installing.clone() });
    }

    /// Search the open skills index. Await the receiver from any executor.
    pub fn search_skills(&self, query: String) -> oneshot::Receiver<anyhow::Result<Vec<SkillListing>>> {
        let (tx, rx) = oneshot::channel();
        let lib = self.skills.clone();
        self.rt.spawn(async move {
            let _ = tx.send(lib.search(&query).await);
        });
        rx
    }

    /// Every skill in a GitHub repo ("owner/name"). Await the receiver from any executor.
    pub fn browse_skills(&self, repo: String) -> oneshot::Receiver<anyhow::Result<Vec<SkillListing>>> {
        let (tx, rx) = oneshot::channel();
        let lib = self.skills.clone();
        self.rt.spawn(async move {
            let _ = tx.send(lib.browse(&repo).await);
        });
        rx
    }

    /// Skills other agents keep on this computer, ready to add to the library.
    pub fn local_agent_skills(&self) -> oneshot::Receiver<Vec<SkillListing>> {
        let (tx, rx) = oneshot::channel();
        self.rt.spawn(async move {
            let found = tokio::task::spawn_blocking(mothr_skills::local_agent_skills).await.unwrap_or_default();
            let _ = tx.send(found);
        });
        rx
    }

    /// Where the library lives on disk.
    pub fn skills_dir(&self) -> PathBuf {
        self.skills.root().to_path_buf()
    }
}
