# Restarts one container. Its name arrives from Hytte as $env:HYTTE_ARG (the row's "arg"):
# treat it as untrusted text, check it, and hand it to docker as a single argument.
$name = $env:HYTTE_ARG
if ($name -notmatch '^[A-Za-z0-9][A-Za-z0-9_.-]*$') { "not a container name"; exit 1 }
docker restart $name | Out-Null
if ($LASTEXITCODE -ne 0) { "could not restart $name"; exit 1 }
"restarted $name"
