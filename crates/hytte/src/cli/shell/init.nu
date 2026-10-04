# Hytte / notch shell integration for Nushell.
# Save the output and source it from config.nu:
#   notch init nu | save -f ~/.config/nushell/notch.nu
#   source ~/.config/nushell/notch.nu
# Commands that run longer than the daemon's threshold (default 3 s) show up in the notch.

$env.NOTCH_SEQ = 0

$env.config = ($env.config | upsert hooks.pre_execution (
    ($env.config.hooks.pre_execution? | default []) | append {||
        $env.NOTCH_SEQ = ($env.NOTCH_SEQ + 1)
        $env.NOTCH_ACTIVE = true
        let id = $"nu-($nu.pid)-($env.NOTCH_SEQ)"
        $env.NOTCH_ID = $id
        job spawn { ^notch hook start --id $id --cmd (commandline) --pid $nu.pid --cwd $env.PWD | ignore } | ignore
    }
))

$env.config = ($env.config | upsert hooks.pre_prompt (
    ($env.config.hooks.pre_prompt? | default []) | append {||
        if ($env.NOTCH_ACTIVE? | default false) {
            $env.NOTCH_ACTIVE = false
            let ms = ($env.CMD_DURATION_MS? | default "0" | into int)
            let code = ($env.LAST_EXIT_CODE? | default 0)
            job spawn { ^notch hook end --id $env.NOTCH_ID --code $code --duration-ms $ms | ignore } | ignore
        }
    }
))
