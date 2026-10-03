# Hytte / notch shell integration for bash (Git Bash, WSL-less MSYS, etc.).
# Add to ~/.bashrc:   eval "$(notch init bash)"
# Commands that run longer than the daemon's threshold (default 3 s) show up in the notch.

if [ -z "${__notch_loaded:-}" ]; then
  __notch_loaded=1
  __notch_seq=0
  __notch_active=
  # Windows pid of this shell (MSYS pids differ from Windows pids).
  __notch_pid=$(cat /proc/$$/winpid 2>/dev/null || echo $$)

  __notch_preexec() {
    [ -n "${__notch_in_prompt:-}" ] && return
    [ -n "${COMP_LINE:-}" ] && return
    case "$BASH_COMMAND" in __notch_*|*PROMPT_COMMAND*) return ;; esac
    [ -n "$__notch_active" ] && return     # only the first command of a line
    __notch_seq=$((__notch_seq + 1))
    __notch_id="sh-$__notch_pid-$__notch_seq"
    __notch_active=1
    __notch_t0=$SECONDS
    ( notch hook start --id "$__notch_id" --cmd "$BASH_COMMAND" --pid "$__notch_pid" --cwd "$PWD" >/dev/null 2>&1 & )
  }

  __notch_precmd() {
    local code=$?
    __notch_in_prompt=1
    if [ -n "$__notch_active" ]; then
      ( notch hook end --id "$__notch_id" --code "$code" --duration-ms "$(((SECONDS - __notch_t0) * 1000))" >/dev/null 2>&1 & )
      __notch_active=
    fi
    __notch_in_prompt=
    return $code
  }

  trap '__notch_preexec' DEBUG
  PROMPT_COMMAND="__notch_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
