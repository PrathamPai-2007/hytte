# Hytte / notch shell integration for zsh.
# Add to ~/.zshrc:   eval "$(notch init zsh)"
# Commands that run longer than the daemon's threshold (default 3 s) show up in the notch.

if [[ -z "${__notch_loaded:-}" ]]; then
  __notch_loaded=1
  typeset -gi __notch_seq=0
  __notch_pid=$(cat /proc/$$/winpid 2>/dev/null || echo $$)
  autoload -Uz add-zsh-hook

  __notch_preexec() {
    __notch_seq=$((__notch_seq + 1))
    __notch_id="zsh-$__notch_pid-$__notch_seq"
    __notch_active=1
    __notch_t0=$EPOCHSECONDS
    ( notch hook start --id "$__notch_id" --cmd "$1" --pid "$__notch_pid" --cwd "$PWD" >/dev/null 2>&1 & )
  }

  __notch_precmd() {
    local code=$?
    if [[ -n "${__notch_active:-}" ]]; then
      ( notch hook end --id "$__notch_id" --code "$code" --duration-ms "$(((EPOCHSECONDS - __notch_t0) * 1000))" >/dev/null 2>&1 & )
      __notch_active=
    fi
    return $code
  }

  zmodload zsh/datetime 2>/dev/null
  add-zsh-hook preexec __notch_preexec
  add-zsh-hook precmd __notch_precmd
fi
