---
name: pr-comments
description: "Fetch and display GitHub PR review comments for the current branch."
---

# Fetch PR Comments

Fetch all review comments from the current branch's GitHub PR and display them via `insert_code_review_comments`.

## Procedure

1. Run the bundled script (must be inside a git repo with an open PR on the current branch).
   Use `do_not_summarize_output: true` when running this shell command so the JSON output is not truncated.
   ```bash
   python3 {{skill_dir}}/scripts/fetch_github_review_comments.py
   ```
   The script prints JSON to stdout.
   If the script fails to fetch comments, run the fallback `gh` commands instead.

2. Call `insert_code_review_comments` with the three top-level fields from the JSON output:
   - `local_repository_path`
   - `base_branch`
   - `comments`

3. Stop and wait for the user. After displaying each batch of comments, you MUST ask the user how they would like to proceed. Do NOT take any further action until the user provides explicit instructions unless the user explicitly asks you to.
Do NOT make code changes in response to the fetched comments unless the user tells you to. Do NOT impersonate the user by submitting review responses.
Your role when fetching and displaying comments is purely informational — present the comments and wait for direction.

## What the Script Handles

- Fetches issue comments, diff comments, and reviews via `gh api --paginate`
- Trims large diff hunks to a window around the commented line
- Sets `reply_metadata` on reply comments
- Sets `location_metadata` on top-level diff comments (filepath, trimmed diff hunk, line, side)
- PR-level comments (issue comments and reviews) have neither location nor reply metadata

## Script fallback commands

If the script fails to fetch comments, follow these steps to fetch comments directly from the GitHub API:

1. Use the GitHub cli to find the PR number, owner name, repo name, and PR base branch for the current branch.

2. Use the GitHub /repos/{owner_login}/{repo_name}/issues/{pr_number}/comments endpoint to fetch PR-level comments.

3. Use the GitHub /repos/{owner_login}/{repo_name}/pulls/{pr_number}/comments endpoint to fetch line- and file-attached review comments. Remove location metadata and diff hunks from thread replies.

4. Use the GitHub /repos/{owner_login}/{repo_name}/pulls/{pr_number}/reviews endpoint with a filter to fetch code reviews with comment text.

5. Invoke the `insert_code_review_comments` tool to send the comments to the user. Include all PR-, review-, file- and line-level comments. If there are no comments on the PR, use the tool to return an empty list. DO NOT read out the comment contents without the tool.

Ensure the pager is not used by clearing the GH_PAGER environment variable. For example, on MacOS using zsh, use:
```sh
$ GH_PAGER="" gh pr view --json number,headRepository,headRepositoryOwner,baseRefName
$ GH_PAGER="" gh api /repos/{owner_login}/{repo_name}/issues/{pr_number}/comments --jq '.[] | {id, html_url, user_login: .user.login, body, created_at, updated_at}'
$ GH_PAGER="" gh api /repos/{owner_login}/{repo_name}/pulls/{pr_number}/comments --jq '.[] | {id, html_url, diff_hunk, path, user_login: .user.login, body, created_at, updated_at, start_line, original_start_line, start_side, line, original_line, side, in_reply_to_id, subject_type} | if .in_reply_to_id != null then del(.diff_hunk, .path, .line, .original_line, .start_line, .original_start_line, .side, .start_side, .subject_type) else . end'
$ GH_PAGER="" gh api /repos/{owner_login}/{repo_name}/pulls/{pr_number}/reviews --jq '.[] | {id, html_url, user_login: .user.login, body, created_at, updated_at} | select(.body != "" and .body != null)'
```
Adapt the instructions above for the user's operating system and shell. Then invoke the `insert_code_review_comments` tool.

6. After displaying comments, follow step 3 of the Procedure above: stop and ask the user how they want to proceed. Do NOT take any action on the comments without explicit user direction.

## Requirements

- `gh` CLI authenticated with repo access
- Current branch has an open pull request
