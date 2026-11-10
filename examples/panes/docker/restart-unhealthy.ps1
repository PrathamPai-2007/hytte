# Restarts every container Docker reports as unhealthy, and says which.
$ids = docker ps --filter "health=unhealthy" --format "{{.Names}}"
if (-not $ids) { "nothing is unhealthy"; exit 0 }
docker restart $ids | Out-Null
"restarted: " + ($ids -join ", ")
