# Hytte 1.9.2 — todo (details in plan.md)

## Phase 1: Reach and capture
- [x] T0 drag the pill sideways (per monitor, free, drag-mode scene)
- [x] T1 `hytte note` / `hytte todo` CLI (no hotkey added)
- [x] T2 WSL spike (works: ship)
- [x] T3 `hytte setup agents --wsl [--undo]`

## Checkpoint A
- [ ] tests + stable clippy clean; `hytte note` works in a dev build

## Phase 2: Agents talk to the pill
- [x] T4 `hytte mcp` stdio server (notify, card, note_add, todo_add, pane_set)
- [x] T5 `hytte setup mcp [--undo]` (doctor line not built)
- [x] T6 agent day summary (in memory only)

## Checkpoint B
- [ ] a real Claude Code session gets a pressed-button answer from `card`
- [ ] Agents pane snapshot reviewed

## Phase 3: Breadth
- [x] T7 Copilot CLI added natively (hooks documented); no data-driven adapter format (YAGNI)
- [x] T8 eight example packs + gallery README

## Phase 4: Ship
- [x] T9 docs, manual, landing FAQ (no GIF)
- [ ] T10 version 1.9.2, test the built app, release on your go-ahead
