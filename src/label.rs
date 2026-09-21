//! Label rendering: template expansion, ticket extraction, and truncation.

/// Values available to a label template.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ctx {
    pub repo: Option<String>,
    pub dir: String,
    pub branch: Option<String>,
    pub ticket: Option<String>,
    pub worktree: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
}

const TOKENS: &[&str] = &[
    "repo", "dir", "branch", "ticket", "worktree", "agent", "status",
];

/// Expand `{token}`s, drop unknown tokens, collapse runs of whitespace, and
/// trim. An all-empty template yields an empty string; callers apply fallback.
pub fn render(template: &str, ctx: &Ctx) -> String {
    let mut out = template.to_string();
    for token in TOKENS {
        let value = match *token {
            "repo" => ctx.repo.as_deref(),
            "dir" => Some(ctx.dir.as_str()),
            "branch" => ctx.branch.as_deref(),
            "ticket" => ctx.ticket.as_deref(),
            "worktree" => ctx.worktree.as_deref(),
            "agent" => ctx.agent.as_deref(),
            "status" => ctx.status.as_deref(),
            _ => None,
        }
        .unwrap_or("");
        out = out.replace(&format!("{{{token}}}"), value);
    }
    strip_unknown_tokens(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove any `{...}` the template left behind so labels never show raw tokens.
fn strip_unknown_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth = 0usize;
    for ch in input.chars() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Attach `…` if the label exceeds `max_length` characters.
pub fn truncate(label: &str, max_length: usize) -> String {
    if max_length == 0 {
        return label.to_string();
    }
    let count = label.chars().count();
    if count <= max_length {
        return label.to_string();
    }
    let keep = max_length.saturating_sub(1);
    let mut out: String = label.chars().take(keep).collect();
    out.push('…');
    out
}

/// Extract the first `PREFIX-NUMBER` ticket from a branch name, e.g.
/// `feature/BACK-565-case-field-updates` -> `BACK-565`.
pub fn extract_ticket(branch: &str, prefixes: &[String]) -> Option<String> {
    let chars: Vec<char> = branch.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if !chars[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < chars.len() && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit()) {
            j += 1;
        }
        let prefix_end = j;
        if j < chars.len() && chars[j] == '-' {
            let mut k = j + 1;
            let digits_start = k;
            while k < chars.len() && chars[k].is_ascii_digit() {
                k += 1;
            }
            let boundary_ok = k == chars.len() || !chars[k].is_ascii_alphanumeric();
            let long_enough = k > digits_start && prefix_end - start >= 2;
            if boundary_ok && long_enough {
                let key: String = chars[start..k].iter().collect();
                let prefix: String = chars[start..prefix_end].iter().collect();
                let allowed =
                    prefixes.is_empty() || prefixes.iter().any(|p| p.eq_ignore_ascii_case(&prefix));
                if allowed {
                    return Some(key.to_ascii_uppercase());
                }
            }
        }
        i = j.max(start + 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx {
            repo: Some("llama-rs".into()),
            dir: "llama-rs".into(),
            branch: Some("feature/BACK-565-thing".into()),
            ticket: Some("BACK-565".into()),
            worktree: Some("BACK-565".into()),
            agent: Some("opencode".into()),
            status: Some("working".into()),
        }
    }

    #[test]
    fn default_template_renders_repo_and_ticket() {
        assert_eq!(render("{repo} {ticket}", &ctx()), "llama-rs BACK-565");
    }

    #[test]
    fn empty_ticket_collapses_whitespace() {
        let mut c = ctx();
        c.ticket = None;
        assert_eq!(render("{repo} {ticket}", &c), "llama-rs");
    }

    #[test]
    fn unknown_tokens_are_dropped() {
        assert_eq!(render("{repo} {nope}", &ctx()), "llama-rs");
    }

    #[test]
    fn all_tokens_expand() {
        let c = ctx();
        assert_eq!(
            render(
                "{repo}|{dir}|{branch}|{ticket}|{worktree}|{agent}|{status}",
                &c
            ),
            "llama-rs|llama-rs|feature/BACK-565-thing|BACK-565|BACK-565|opencode|working"
        );
    }

    #[test]
    fn missing_fields_render_empty() {
        let c = Ctx {
            dir: "scratch".into(),
            ..Ctx::default()
        };
        assert_eq!(render("{repo} {ticket} {dir}", &c), "scratch");
    }

    #[test]
    fn truncation_uses_ellipsis() {
        assert_eq!(truncate("llama-rs BACK-565", 0), "llama-rs BACK-565");
        assert_eq!(truncate("llama-rs BACK-565", 8), "llama-r…");
        assert_eq!(truncate("short", 8), "short");
    }

    #[test]
    fn ticket_from_various_branches() {
        let none: &[String] = &[];
        assert_eq!(
            extract_ticket("feature/BACK-565-thing", none).as_deref(),
            Some("BACK-565")
        );
        assert_eq!(extract_ticket("develop", none), None);
        assert_eq!(extract_ticket("main", none), None);
        // Jira keys are uppercase by convention; lowercase is not a ticket.
        assert_eq!(extract_ticket("fix/lit-42", none), None);
        assert_eq!(extract_ticket("release-2026", none), None);
    }

    #[test]
    fn ticket_prefix_allowlist() {
        let only_back = vec!["BACK".to_string()];
        assert_eq!(
            extract_ticket("feature/BACK-565-x", &only_back).as_deref(),
            Some("BACK-565")
        );
        // UTF-8 shaped tokens are rejected when the allowlist excludes them.
        assert_eq!(extract_ticket("fix/UTF-8", &only_back), None);
    }
}
