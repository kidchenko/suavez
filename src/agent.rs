//! The coding agents Sua Vez listens to.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    /// Session files written before Sua Vez knew about other agents have no
    /// agent field, and they were all Claude Code.
    #[default]
    Claude,
    Codex,
    Gemini,
    Devin,
}

impl Agent {
    pub const ALL: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Gemini, Agent::Devin];

    /// Who does the work, for "Codex finished after 4m."
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini",
            Agent::Devin => "Devin",
        }
    }

    /// The tool you install, for "Connect to Gemini CLI".
    pub fn product(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
            Agent::Devin => "Devin",
        }
    }

    /// The `--agent` value and the session file prefix.
    pub fn slug(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
            Agent::Devin => "devin",
        }
    }

    pub fn parse(s: &str) -> Option<Agent> {
        Agent::ALL
            .into_iter()
            .find(|a| a.slug().eq_ignore_ascii_case(s))
    }

    /// Codex and Gemini read a hook's stdout as JSON and reject an empty one.
    /// Claude Code and Devin are content with nothing, and for some of their
    /// events stdout becomes context for the model, so they get nothing.
    pub fn wants_json_reply(self) -> bool {
        matches!(self, Agent::Codex | Agent::Gemini)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip() {
        for a in Agent::ALL {
            assert_eq!(Agent::parse(a.slug()), Some(a));
        }
        assert_eq!(Agent::parse("GEMINI"), Some(Agent::Gemini));
        assert_eq!(Agent::parse("gpt"), None);
    }
}
