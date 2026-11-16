# Docker

Containers with their status, unhealthy and stopped ones first.

- An **unhealthy** row is a button. It carries the container's name as its `arg`, so clicking it runs the `restart` action for that one container (`restart.ps1` reads the name from `$env:HYTTE_ARG`, checks it, and restarts it). It asks once more (`Sure?`) first.
- When anything is unhealthy a **Fix unhealthy** button appears under the rows. It runs `restart-unhealthy` (also after `Sure?`), which restarts every unhealthy container and says which.

Both actions show a result card when they finish.

Needs Docker Desktop. With the engine stopped the pane says "Docker isn't running".
