# Example pane packs

A pack is a folder with a `pane.toml` (and the scripts it runs). Copy one into
`%APPDATA%\Hytte\panes\` and Hytte shows a card with the command and **Run** / **Ignore**; nothing runs
until you press Run. Edit the folder later and it asks again.

| Pack | Shows | Needs |
|---|---|---|
| [`github-prs`](github-prs) | your open pull requests (a click opens github.com/pulls) | the [GitHub CLI](https://cli.github.com) (`gh`), signed in |
| [`docker`](docker) | containers, unhealthy ones first, with a confirmed **restart unhealthy** action | Docker Desktop running |
| [`ci-status`](ci-status) | the latest GitHub Actions runs of one repository | `gh`, and `repo.txt` (copy `repo.txt.example`, put your `owner/name` in it) |

```powershell
Copy-Item -Recurse .\github-prs "$env:APPDATA\Hytte\panes\"
```

Each script prints `{"rows": [...]}` (see the manual's *Everyday tools* section): up to six rows with
`text`, `value`, `progress`, `tone` (`good`, `warn`, `bad`) and `action`. A row's `action` is the
name of an `[[action]]` in the same `pane.toml`; Hytte prefixes it with the folder's name. If a tool
is missing the script prints a row that says so instead of failing.
