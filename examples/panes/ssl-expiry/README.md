# TLS certificates

Days left on the HTTPS certificate of each host in `hosts.txt` (one per line, `host` or `host:port`, six at most). Amber below 30 days, red below 7 or already expired. Copy `hosts.txt.example` to `hosts.txt` first. It connects only to the hosts you list, once an hour, and sends nothing but the TLS handshake.
