# Allow / Deny for Gemini CLI, Cursor and Antigravity: findings (2026-11-10)

Question: can the pill answer a permission prompt for these agents the way it does for Claude Code,
Codex and OpenCode? **Result: no, for none of the three. Nothing was built.**

| Agent | What its hooks can do | Why Allow / Deny doesn't fit |
|---|---|---|
| Gemini CLI | `BeforeTool` returns `{"decision": "allow" \| "deny", "reason": ...}` ([hooks reference](https://geminicli.com/docs/hooks/reference/)) | The hook fires before **every** tool call, including the ones Gemini runs without asking. There is no hook for "a permission prompt is about to show" (`Notification` is informational). Asking on every call would flood the pill and approve things Gemini would have run anyway. |
| Cursor | `beforeShellExecution` returns `permission: allow \| deny \| ask` | Cursor staff say only `deny` is enforced; `allow` and `ask` are ignored in several versions ([forum](https://forum.cursor.com/t/hooks-ask-permission-broken-in-2-4-21/150020)), and malformed output defaults to allow ([forum](https://forum.cursor.com/t/beforeshellexecution-hook-malformed-json-response-silently-allows-command-instead-of-blocking/152669)). An Allow button that does nothing is worse than none. |
| Antigravity | `PreToolUse` (and the Gemini API `pre_tool_execution`) can approve or deny from JSON on stdout ([docs](https://antigravity.google/docs/hooks), [agent hooks](https://ai.google.dev/gemini-api/docs/agent-hooks)) | Same shape as Gemini CLI: it runs per tool call, not per prompt, and the exact output schema could not be confirmed from the docs. |

Revisit when an agent adds a hook that fires only when a prompt would show (Claude Code's
`PermissionRequest` is the model), or when Cursor enforces `allow`.
