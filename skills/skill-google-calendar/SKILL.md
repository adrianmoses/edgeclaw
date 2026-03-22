---
name: skill-google-calendar
description: Manage Google Calendar events — list, create, update, delete events. Use when the user asks about their calendar, scheduling meetings, checking availability, or managing events. Part of the google_workspace MCP skill.
metadata:
  transport: mcp
  mcp_server: http://workspace-mcp:8000/mcp
  skill_group: google_workspace
  destructive_tools:
    - manage_event
---

## Tools

### list_calendars

List all calendars accessible to the authenticated user. Returns calendar name, id, and access role.

### get_events

List events from a calendar. Supports filtering by time range and search query. Use this to check what's on the user's schedule or to find available time slots by examining gaps between events.

### manage_event

Create, update, or delete a calendar event. **Destructive — requires approval.**

Use this for all event modifications:
- To create: provide calendar_id, summary, start, end, and optional description/location/attendees
- To update: provide event_id along with fields to change
- To delete: provide event_id with a delete action

## Finding Free Time

To find available time slots, use `get_events` to fetch events in the desired time range, then identify gaps between events. Consider the user's working hours (typically 9am-5pm) when suggesting available slots.

## Error Handling

- **Auth errors:** The MCP server handles token refresh automatically. If auth fails completely, the tool returns an error indicating re-authorization is needed.
- **404 Not Found:** Calendar or event ID does not exist.
- **409 Conflict:** Event was modified concurrently. Retry with updated event.
