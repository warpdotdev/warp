# Restore the user's startup directory before invoking their interactive config.
if [[ -n ${WARP_SWIFT_ORIGINAL_ZDOTDIR+x} ]]; then
  export ZDOTDIR=$WARP_SWIFT_ORIGINAL_ZDOTDIR
else
  unset ZDOTDIR
fi
unset WARP_SWIFT_ORIGINAL_ZDOTDIR

typeset -gr _warp_swift_session=$WARP_SWIFT_SESSION
typeset -gr _warp_swift_token=$WARP_SWIFT_TOKEN
unset WARP_SWIFT_SESSION WARP_SWIFT_TOKEN
typeset -gr _warp_swift_marker="WS${_warp_swift_token[1,8]}-$$-"

if [[ -r ${ZDOTDIR-$HOME}/.zshrc ]]; then
  source "${ZDOTDIR-$HOME}/.zshrc"
fi

autoload -Uz add-zsh-hook add-zle-hook-widget

_warp_swift_precmd() {
  local exit_code=$?
  if [[ $_warp_swift_running == 1 ]]; then
    printf '\n\e[0;8m%sE\e[0m\n' "$_warp_swift_marker"
    _warp_swift_running=0
  fi
  printf '\e]133;D;%d\a\e]133;A\a' "$exit_code"
  printf '\e]2;warp-swift:%s:done:%d:%d:%s\a' "$_warp_swift_token" "$$" "$exit_code" "${(V)PWD}"
  return 0
}

_warp_swift_preexec() {
  _warp_swift_running=1
  printf '\e[0;8m%sS\e[0m\n' "$_warp_swift_marker"
  printf '\e]133;C\a\e]2;warp-swift:%s:busy:%d\a' "$_warp_swift_token" "$$"
}

_warp_swift_ready() {
  printf '\e]133;B\a\e]2;warp-swift:%s:ready\a' "$_warp_swift_token"
}

_warp_swift_submit() {
  local command_file="$_warp_swift_session/command"
  [[ -f $command_file ]] || return 0
  # Read through EOF, preserving embedded and trailing newlines.
  IFS= read -r -d $'\0' BUFFER < "$command_file"
  command rm -f -- "$command_file"
  CURSOR=${#BUFFER}
  zle .accept-line
}

_warp_swift_complete_candidate() {
  _main_complete
  _warp_swift_match_count=${compstate[nmatches]:-0}
  compstate[list]=''
  if (( _warp_swift_match_count > 1 )); then
    compstate[insert]=$_warp_swift_match_index
  fi
}

_warp_swift_complete() {
  setopt localoptions COMPLETE_IN_WORD MULTIBYTE EXTENDED_GLOB
  local request="$_warp_swift_session/completion-request"
  [[ -f $request ]] || return 0
  local request_id cursor draft
  {
    IFS= read -r request_id
    IFS= read -r cursor
    IFS= read -r -d $'\0' draft
  } < "$request"
  command rm -f -- "$request"
  [[ $request_id == [0-9A-F-]## && ${#request_id} == 36 && $cursor == <0-8192> ]] || return 0

  local original_buffer=$BUFFER original_cursor=$CURSOR
  local -i _warp_swift_match_count _warp_swift_match_index
  if (( ! $+functions[compdef] )); then
    autoload -Uz compinit
    compinit -D -i
  fi
  # Ask ZLE to produce the full edited buffer for each candidate. That preserves
  # quoting, any suffix append rules, and the position inside a partially typed word.
  {
    for _warp_swift_match_index in {1..6}; do
      BUFFER=$draft
      CURSOR=$cursor
      _warp_swift_match_count=0
      zle _warp_swift_candidate_widget >/dev/null
      (( _warp_swift_match_count >= _warp_swift_match_index )) || break
      printf '%d\0%s\0' "$CURSOR" "$BUFFER"
      (( _warp_swift_match_count > _warp_swift_match_index )) || break
    done
  } > "$_warp_swift_session/completion-$request_id"
  BUFFER=$original_buffer
  CURSOR=$original_cursor
  printf '\e]2;warp-swift:%s:completed:%s\a' "$_warp_swift_token" "$request_id"
  return 0
}

precmd_functions=(_warp_swift_precmd "${precmd_functions[@]}")
add-zsh-hook preexec _warp_swift_preexec
add-zle-hook-widget line-init _warp_swift_ready
zle -N _warp_swift_submit
zle -N _warp_swift_complete
zle -C _warp_swift_candidate_widget complete-word _warp_swift_complete_candidate
for _warp_swift_keymap in emacs viins vicmd; do
  bindkey -M "$_warp_swift_keymap" $'\e[98~' _warp_swift_submit
  bindkey -M "$_warp_swift_keymap" $'\e[99~' _warp_swift_complete
done
unset _warp_swift_keymap

warpify-zsh() {
  local -x WARP_SWIFT_SESSION=$_warp_swift_session
  local -x WARP_SWIFT_TOKEN=$_warp_swift_token
  local -x WARP_SWIFT_ORIGINAL_ZDOTDIR=${ZDOTDIR-$HOME}
  local -x ZDOTDIR=$_warp_swift_session
  /bin/zsh -i "$@"
}
