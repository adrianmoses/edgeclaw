---
name: skill-github
description: Interact with GitHub repositories, issues, and pull requests. Use when the user asks about repos, issues, PRs, commits, or code on GitHub.
metadata:
  transport: mcp
  mcp_server: https://api.githubcopilot.com/mcp
  destructive_tools:
    - issue_write
    - create_pull_request
    - merge_pull_request
    - add_issue_comment
    - create_or_update_file
    - push_files
---

## Tools

### issue_read

Read a single issue by owner, repo, and issue number. Returns title, body, labels, state, assignees, and comments.

### issue_write

Create or update an issue. **Destructive — requires approval.**

### list_issues

List issues for a repository, with optional filters for state, labels, assignee, and pagination.

### search_issues

Search issues and pull requests across repositories using GitHub's search syntax.

### list_pull_requests

List pull requests for a repository, with optional filters for state, head, base, and sort.

### pull_request_read

Read a single pull request with full details including diff stats, review status, and merge state.

### create_pull_request

Create a new pull request. **Destructive — requires approval.**

### merge_pull_request

Merge a pull request. **Destructive — requires approval.**

### add_issue_comment

Add a comment to an issue or pull request. **Destructive — requires approval.**

### search_code

Search for code across repositories using GitHub code search syntax.

### search_repositories

Search for repositories by name, description, language, or other criteria.

### get_file_contents

Get the contents of a file or directory from a repository.

### list_commits

List commits for a repository, optionally filtered by path, author, or branch.

### create_or_update_file

Create or update a file in a repository. **Destructive — requires approval.**

### push_files

Push multiple files to a repository in a single commit. **Destructive — requires approval.**

### list_branches

List branches for a repository.

### get_me

Get details about the authenticated GitHub user.

## Error Handling

- **401 Unauthorized:** Token invalid or revoked.
- **403 Forbidden:** Insufficient scopes or rate limit exceeded.
- **422 Unprocessable Entity:** Validation error (missing required fields, invalid values).
- **Rate limits:** GitHub surfaces `X-RateLimit-Remaining` and `X-RateLimit-Reset` in responses.
