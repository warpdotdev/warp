use std::collections::HashMap;

pub(crate) const WORKTREE_LIST_SEPARATOR: &str = "\u{1e}";

const ENCODED_VALUE_SEPARATOR: char = '\u{1f}';
const WORKTREE_TAG: &str = "worktree";
const GIT_BRANCH_REF_PREFIX: &str = "refs/heads/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitBranchOnClickValue {
    pub(crate) branch_name: String,
    pub(crate) worktree_path: Option<String>,
    pub(crate) is_linked_worktree: bool,
}

impl GitBranchOnClickValue {
    pub(crate) fn new(branch_name: String) -> Self {
        Self {
            branch_name,
            worktree_path: None,
            is_linked_worktree: false,
        }
    }

    fn linked_worktree(branch_name: String, worktree_path: Option<String>) -> Self {
        Self {
            branch_name,
            worktree_path,
            is_linked_worktree: true,
        }
    }

    pub(crate) fn encode(&self) -> String {
        if self.is_linked_worktree {
            match &self.worktree_path {
                Some(path) => format!(
                    "{}{ENCODED_VALUE_SEPARATOR}{WORKTREE_TAG}{ENCODED_VALUE_SEPARATOR}{path}",
                    self.branch_name
                ),
                None => format!(
                    "{}{ENCODED_VALUE_SEPARATOR}{WORKTREE_TAG}",
                    self.branch_name
                ),
            }
        } else {
            self.branch_name.clone()
        }
    }

    pub(crate) fn decode(value: &str) -> Self {
        let mut parts = value.splitn(3, ENCODED_VALUE_SEPARATOR);
        let branch_name = parts.next().unwrap_or_default().to_string();

        match parts.next() {
            Some(WORKTREE_TAG) => {
                let worktree_path = parts
                    .next()
                    .filter(|path| !path.is_empty())
                    .map(str::to_string);
                Self::linked_worktree(branch_name, worktree_path)
            }
            _ => Self::new(branch_name),
        }
    }
}

struct ParsedGitBranchLine {
    branch_name: String,
    is_current: bool,
    is_linked_worktree: bool,
}

pub(crate) fn filter_git_branch_on_click_values(
    values_opt: Option<Vec<String>>,
) -> Option<Vec<String>> {
    values_opt.map(|values| {
        let worktree_list_separator_index = values
            .iter()
            .position(|value| value.trim() == WORKTREE_LIST_SEPARATOR);

        let (branch_lines, worktree_lines) = match worktree_list_separator_index {
            Some(index) => (&values[..index], &values[index + 1..]),
            None => (&values[..], &[][..]),
        };

        let branch_to_worktree_path = parse_git_worktree_paths(worktree_lines);
        let branches: Vec<ParsedGitBranchLine> = branch_lines
            .iter()
            .filter_map(|line| parse_git_branch_line(line))
            .collect();

        // Keep the current branch first (denoted by *), preserving relative order
        // for the remaining branches.
        let (current_branches, other_branches): (Vec<_>, Vec<_>) =
            branches.into_iter().partition(|branch| branch.is_current);

        current_branches
            .into_iter()
            .chain(other_branches)
            .map(|branch| {
                if branch.is_linked_worktree {
                    GitBranchOnClickValue::linked_worktree(
                        branch.branch_name.clone(),
                        branch_to_worktree_path.get(&branch.branch_name).cloned(),
                    )
                } else {
                    GitBranchOnClickValue::new(branch.branch_name)
                }
                .encode()
            })
            .collect()
    })
}

fn parse_git_branch_line(line: &str) -> Option<ParsedGitBranchLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let status_marker = ['*', '+'].into_iter().find_map(|marker| {
        trimmed.strip_prefix(marker).and_then(|rest| {
            rest.chars()
                .next()
                .filter(|c| c.is_whitespace())
                .map(|_| marker)
        })
    });

    let branch_name = match status_marker {
        Some(marker) => trimmed
            .strip_prefix(marker)
            .map(str::trim)
            .unwrap_or(trimmed),
        None => trimmed,
    };

    if branch_name.is_empty() {
        return None;
    }

    Some(ParsedGitBranchLine {
        branch_name: branch_name.to_string(),
        is_current: status_marker == Some('*'),
        is_linked_worktree: status_marker == Some('+'),
    })
}

fn parse_git_worktree_paths(lines: &[String]) -> HashMap<String, String> {
    let mut branch_to_worktree_path = HashMap::new();
    let mut current_worktree_path: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            current_worktree_path = None;
            continue;
        }

        if let Some(path) = trimmed.strip_prefix("worktree ") {
            current_worktree_path = Some(path.to_string());
            continue;
        }

        let Some(branch_ref) = trimmed.strip_prefix("branch ") else {
            continue;
        };
        let Some(branch_name) = branch_ref.strip_prefix(GIT_BRANCH_REF_PREFIX) else {
            continue;
        };
        let Some(path) = current_worktree_path.as_ref() else {
            continue;
        };

        branch_to_worktree_path.insert(branch_name.to_string(), path.clone());
    }

    branch_to_worktree_path
}

/// Returns `true` when `name` looks like a plausible git branch name that can
/// be created via `git checkout -b`.
///
/// We err on the side of letting git itself reject borderline cases: this
/// helper only filters out the most obviously broken inputs so that the
/// "Create new branch …" affordance does not appear for clearly invalid
/// queries (e.g. an empty string after the user backspaces, or whitespace).
/// Anything we accept here may still be rejected by `git check-ref-format`,
/// in which case the user sees the failure in the terminal.
pub(crate) fn is_plausible_new_branch_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    // git rejects names beginning with `-` outright, and they would also be
    // ambiguous with `git checkout -b` flags, so don't offer the affordance.
    if trimmed.starts_with('-') {
        return false;
    }
    // git refuses whitespace (other than as a separator) inside refs.
    if trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    true
}

#[cfg(test)]
#[path = "git_branch_on_click_tests.rs"]
mod tests;
