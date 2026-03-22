---
name: skill-gmail
description: Read, search, and send Gmail messages. Use when the user asks about email, inbox, unread messages, or sending mail. Part of the google_workspace MCP skill.
metadata:
  transport: mcp
  mcp_server: http://workspace-mcp:8000/mcp
  skill_group: google_workspace
  destructive_tools:
    - send_gmail_message
---

## Tools

### search_gmail_messages

Search for Gmail messages using Gmail search syntax (same as the Gmail search box). Returns message summaries including id, threadId, snippet, from, to, subject, and date.

### get_gmail_message_content

Get the full content of a single Gmail message by ID. Returns headers, body text, and attachment metadata.

### get_gmail_messages_content_batch

Get the full content of multiple Gmail messages by their IDs in a single call. More efficient than calling get_gmail_message_content repeatedly.

### send_gmail_message

Send an email message. **Destructive — requires approval.**

Parameters include to, subject, body, and optional cc/bcc.

## Error Handling

- **Auth errors:** The MCP server handles token refresh automatically. If auth fails completely, the tool returns an error indicating re-authorization is needed.
- **Rate limits:** Gmail API rate limits are surfaced in error responses.
