# Note that WARP_SESSION_ID is expected to have been set when executing commands to
# emit the InitShell payload, which includes the session ID.
#
# Throughout, command -p is used to call external binaries. command -p resolves the
# given command using the system default $PATH, which ensures the shells can locate
# the corresponding binaries even if the user has a clobbered value of $PATH.
if [ -z "$WARP_BOOTSTRAPPED" ]; then
    # Byte sequence used to signal the start of a DCS. ([0x1b, 0x50, 0x24] which
    # maps to <ESC>, P, $ in ASCII.)
    DCS_START="$(printf '\eP$')"

    # Appended to $DCS_START to signal that the following message is JSON-encoded.
    DCS_JSON_MARKER="d"

    # Byte used to signal the end of a DCS.
    DCS_END="$(printf '\x9c')"

    OSC_START="$(printf '\e]9278;')"

    OSC_END="$(printf '\a')"

    OSC_PARAM_SEPARATOR=";"

    RESET_GRID_OSC="$(printf '\e]9279\a')"

    # OSC used to mark the start of in-band command output.
    #
    # Printable characters received this OSC and OSC_END_GENERATOR_OUTPUT are parsed and handled as
    # output for an in-band command.
    OSC_START_GENERATOR_OUTPUT="$(printf '\e]9277;A\a')"

    # OSC used to mark the end of in-band command output.
    #
    # Printable characters received between OSC_START_GENERATOR_OUTPUT and this are parsed and
    # handled as output for an in-band command.
    OSC_END_GENERATOR_OUTPUT="$(printf '\e]9277;B\a')"

    # Attempt to cd to the desired initial working directory, swallowing any
    # errors.  If this fails, the user will end up in their home directory.
    if [[ ! -z "$WARP_INITIAL_WORKING_DIR" ]]; then
        cd "$WARP_INITIAL_WORKING_DIR" >/dev/null 2>&1
        unset WARP_INITIAL_WORKING_DIR
    fi

    # We configure history to `ignorespace` to avoid leaking our bootstrap script
    # into the user's history. At this point, we unset this option because we don't
    # need it anymore and to avoid side effects.
    # Note that bash-preexec will change this setting for its functionality and
    # clobbers `ignorespace`.
    unset HISTCONTROL

    define_bashpreexec_functions

    # The temporary files used to track generator PIDs.  We'll fill these in later,
    # if we execute any generator commands.
    _WARP_GENERATOR_PIDS_STARTED_TMP_FILE=""
    _WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE=""
    # Make sure we delete generator PID files when the shell exits, if they exist.
    __warp_generator_pid_file_cleanup() {
      if [[ -f $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE ]]; then
        command -p rm $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE
      fi
      if [[ -f $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE ]]; then
        command -p rm $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE
      fi
    }
    trap __warp_generator_pid_file_cleanup EXIT

    # Writes a hex-encoded JSON message to the pty.
    warp_send_json_message () {
        # Sends a message to the controlling terminal as a DSC control sequence.
        # Note that because the JSON string may contain characters that we don't control (including
        # unicode), we encode it as hexadecimal string to avoid prematurely calling unhook if
        # one of the bytes in JSON is 9c (ST) or other (CAN, SUB, ESC).
        encoded_message=$(warp_hex_encode_string "$1")
        # We send the InitShell hook via OSCs when on WSL or MSYS2 or SSH from Windows and via DCSs otherwise.
        # Note that $WARP_USING_WINDOWS_CON_PTY is set in the init shell script.
        if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then
          printf $OSC_START$DCS_JSON_MARKER$OSC_PARAM_SEPARATOR$encoded_message$OSC_END
        else
          printf $DCS_START$DCS_JSON_MARKER$encoded_message$DCS_END
        fi
    }

    # Emit the ExitShell hook right before the remote shell exits so the Warp
    # client can drop per-session resources (specifically the
    # `ssh … remote-server-proxy` child that holds a multiplexed channel on
    # the foreground ssh ControlMaster). This avoids a hang where the master
    # waits on orphaned slave channels when the user ends their interactive
    # session.
    #
    # Only relevant for remote SSH shells. WARP_IS_SSH is exported to "1"
    # by `warp_ssh_helper` on the remote side of a Warp-managed SSH session
    # and is unset everywhere else (local shells, subshells, docker
    # sandboxes, etc.), so the hook only fires where a remote-server-proxy
    # actually needs tearing down.
    #
    # Installed after warp_send_json_message is defined so the handler is
    # callable the moment the trap is registered.
    if [[ "$WARP_IS_SSH" == "1" ]]; then
        __warp_emit_exit_shell() {
            if [[ -n "$WARP_SESSION_ID" ]]; then
                warp_send_json_message \
                    "{\"hook\": \"ExitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID}}"
            fi
        }
        # Bash allows only one handler per signal, so compose with the
        # already-installed generator cleanup. Cover both normal exit (exit,
        # logout, Ctrl-D) and SIGHUP (connection drop).
        __warp_on_exit() {
            __warp_emit_exit_shell
            __warp_generator_pid_file_cleanup
        }
        trap __warp_on_exit EXIT HUP
    fi

    warp_maybe_send_reset_grid_osc () {
        if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then
            printf $RESET_GRID_OSC
        fi
    }

    # Expects the first argument to be the shell hook.
    warp_send_hook_via_kv_pairs_start () {
      printf "${OSC_START}k;A;%s\a" $1
    }

    # Expects the first argument to be the key and the second argument to be the value.
    warp_send_hook_kv_pair_escaped () {
      # Note that we only escape the value.
      if [[ -n "$2" ]]; then
        printf "${OSC_START}k;B;%s;%q\a" "$1" "$2"
      else
        # Don't print anything for the empty value.
        printf "${OSC_START}k;B;%s;\a" "$1"
      fi
    }

    # Expects the first argument to be the key and the second argument to be the value.
    warp_send_hook_kv_pair () {
      # Note that we only escape the value.
      if [[ -n "$2" ]]; then
        printf "${OSC_START}k;B;%s;%s\a" "$1" "$2"
      else
        # Don't print anything for the empty value.
        printf "${OSC_START}k;B;%s;\a" "$1"
      fi
    }

    warp_send_hook_via_kv_pairs_end () {
      printf "${OSC_START}k;C\a"
    }

    # Hex-encodes the given argument and writes it to the PTY, wrapped in the OSC
    # sequences for generator output.
    #
    #
    # Usage:
    #   warp_send_generator_output_osc $my_message
    #
    # The payload of the OSC is "<content_length>;<hex-encoded content>".
    warp_send_generator_output_osc () {
        local hex_encoded_message=$(warp_hex_encode_string "$1")
        warp_send_generator_output_osc_pre_hex_encoded "$hex_encoded_message"
    }

    # Note: If we're on windows, we send a reset grid to erase any cursor mutations caused by
    # the in-band command.
    warp_send_generator_output_osc_pre_hex_encoded () {
        local byte_count=$(LC_ALL="C"; printf "${#1}")
        printf "%b%i;%s%b" $OSC_START_GENERATOR_OUTPUT $byte_count $1 $OSC_END_GENERATOR_OUTPUT
        warp_maybe_send_reset_grid_osc
    }


    # Executes the given command and writes its output to the pty wrapped in a
    # DCS.  The written DCS conforms to a basic schema including other metadata:
    #   "<command_id>;<command_output>;<exit_code>"
    # where command_id is the ID given as the first argument to this function,
    # exit_code is the exit code of the executed command, and command_output is
    # the output itself.
    _warp_execute_command() {
      local command_id=$1
      # This is shorthand to slice the 2nd-nth arguments of this function (i.e.
      # the command array) into its own array. The first argument is the
      # command_id stored above.
      #
      # This must be double-quoted to prevent bash word-splitting, which would effectively replace
      # newlines and tabs with spaces, potentially invalidating the syntactical correctness of the
      # command.
      local command="${@:2}"
      # Bash cannot handle null characters in variables or command substitutions, so hex encode the
      # output immediately before it's stored anywhere. This hex encoding must be done inline --
      # bash doesn't like functions called with null bytes either.
      local generator_output="$( {
        echo -n "$command_id;";
      # Command substitution only captures stdout, so redirect stderr to stdout.
        eval "$command" 2>&1;
        echo -n ";$?";
      } | command -p od -An -v -tx1 | command -p tr -d ' \n')"
      warp_send_generator_output_osc_pre_hex_encoded "$generator_output"
    }

    # Runs the given command in the background, records its PID in
    # _WARP_GENERATOR_PIDS_STARTED_TMP_FILE, and adds its PID from the file when
    # the job is completed.
    _warp_run_generator_command_internal() {
      # $@ must be double-quoted to prevent word-splitting, which would cause the given command to
      # be split into a bash list on $IFS chars (spaces, tabs, newlines), which could invalidate
      # the syntactical correctness of the command.
      _warp_execute_command "$@" &
      # $! contains the PID of the most recently backgrounded command.
      local pid=$!
      echo $pid >> $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE
      wait $pid 2> /dev/null

      # If the exit code of the backgrounded _warp_execute_command process is non-zero,
      # the call to send the generator output failed (most likely because this is being
      # executed in an old bash version that doesn't support some syntax in
      # _warp_execute_command function itself). In this case, send empty output with
      # exit code 1 to indicate generator execution failed.
      if [[ $? -ne 0 ]]; then
          warp_send_generator_output_osc "$1;;1"
      fi


      # Add the PID to the completed generators PID file.
      # 
      # The completed generator PIDs file may not exist if this generator was (by
      # error) left running/not cancelled properly in warp_preexec.
      if [[ -f $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE ]]; then
        echo $pid >> $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE
      fi
    }

    # Executes a generator command in the background, where the first argument is
    # the "command ID" assigned by the Rust app and the second-nth arguments
    # specify the command to be run.
    #
    # Note that the command string should be single-quoted so any variables are
    # not substituted until the command string is actually evaluated.
    #
    # Usage:
    #   warp_run_generator_command <command_id> '<command> <arg1> ... <argn>'
    warp_run_generator_command() {
      # Setting this environment variable prevents warp_precmd from emitting the
      # 'Block started' hook to the Rust app.
      _WARP_GENERATOR_COMMAND=1

      # Ensure the started and completed generator PID files exist.
      if [[ -z $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE || ! -f $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE ]]; then
        _WARP_GENERATOR_PIDS_STARTED_TMP_FILE="$(command -p mktemp)"
      fi
      if [[ -z $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE || ! -f $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE ]]; then
        _WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE="$(command -p mktemp)"
      fi

      # To minimize latency and prevent the user from being blocked from entering a command,
      # cache the user's precmd_functions and only register warp_precmd. In the warp_precmd
      # execution following this generator command, the user's precmd_functions are restored.
      _USER_PRECMD_FUNCTIONS=(${precmd_functions[@]})
      precmd_functions=(warp_precmd)

      # $@ must be double-quoted to prevent word-splitting, which would cause the given command to
      # be split into a bash list on $IFS chars (spaces, tabs, newlines), which could invalidate
      # the syntactical correctness of the command.
      (_warp_run_generator_command_internal "$@" &)
    }


    # Note that this is very performance sensitive code, so try not to
    # invoke any external commands in here.
    warp_preexec () {
        # Use the $BASH_COMMAND environment variable instead of $1, which is passed in by bash_preeexec.
        #
        # Bash_preexec intends to pass the command to preexec functions (as $1), but it utilizes session
        # history to do so. This means that $1 is not the correct command if the executed command is ignored
        # by history (e.g. via $HISTCONTROL or $HISTIGNORE); for example, all in-band generators are ignored
        # by history.
        if [ "$WARP_IN_MSYS2" = true ]; then
          warp_send_hook_via_kv_pairs_start "Preexec"
          warp_send_hook_kv_pair "command" "$BASH_COMMAND"
          warp_send_hook_via_kv_pairs_end
        else
          local truncated_command=$(warp_escape_json "$BASH_COMMAND")
          warp_send_json_message "{\"hook\": \"Preexec\", \"value\": {\"command\": \"$truncated_command\"}}"
        fi
        warp_maybe_send_reset_grid_osc


        # Since we did not early-return above, this hook is for a user-entered
        # command. Kill ongoing generator jobs so their output does not interfere
        # with the user command's output.
        if [[ "$BASH_COMMAND" != warp_run_generator_command* ]] && [[ -f $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE ]] && [[ -f $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE ]]
        then
          # Read PIDs from the started generators tmp file that are not present in
          # the completed generators tmp file into a bash array.
          #
          # The logic used to be the following:
          #
          # pids=($(command -p comm -23 $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE))
          #
          # However, that requires that the files are sorted, which we do not enforce (the OS can assign PIDs
          # in any order).  While we could sort the files and then compare them, the files are expected to be
          # small, so we avoid the overhead of spawning multiple processes and instead do the comparison
          # manually.
          completed_pids=()
          while IFS= read -r pid; do
            completed_pids+=("$pid")
          done < $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE

          pids=()
          while IFS= read -r pid; do
            found=0
            for completed_pid in "${completed_pids[@]}"; do
              if [[ "$pid" == "$completed_pid" ]]; then
                found=1
                break
              fi
            done
            if (( found == 0 )); then
              pids+=("$pid")
            fi
          done < $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE

          # If the array is not empty, kill the ongoing pids.
          if [[ ! -z $pids ]]; then
            # Suppress stderr output; kill writes to stderr if any of the given
            # PIDS are not running (which might rarely be the case due to race
            # conditions in checking which PIDS to cancel and this kill command.
            kill -9 $pids >/dev/null 2>/dev/null
          fi 
        fi
    }

    # Set terminal window and tab title to the same title value. Note that for values longer than 25
    # characters, we truncate the title and prepend "..".
    # Usage warp_title "title"
    # Users can disable the auto title if they chose to by setting WARP_DISABLE_AUTO_TITLE.
    warp_title () {
      DISABLE_AUTO_TITLE="1"

      # truncating the title's len to 25 characters and leading ".."
      tmp_len=$((${#1}-25)) # starting character's position
      len=$((tmp_len>0 ? tmp_len : 0)) # account for the shorter strings
      if [[ $len -ne 0 ]]; then
        title="..${1:$len}" # shorten the argument and prepend the leading ".."
      else
        title="$1"
      fi
      # Set the title. Be sure to make the title a %s argument to prevent title content from ending up
      # in the block output, see:
      # https://linear.app/warpdotdev/issue/WAR-6064/bash-commands-having-esc-write-the-command-to-the-block-output
      printf "\033]0;%s\a" "$title"
    }

    # Runs before executing the command
    warp_set_title_idle_on_precmd () {
      # If the user wants to set the title themselves, they can set the WARP_DISABLE_AUTO_TITLE flag.
      if [ ! -z "$WARP_DISABLE_AUTO_TITLE" ]; then
        return
      fi

      # Note that in older versions of bash (the one builtin with MacOS) the `~` character is just that, 
      # however, when used within `""` (double quotes) on a newer (homebrew) bash version,
      # it automatically EXPANDS to the actual value of $HOME and needs to be escaped to give a proper
      # tilde character. So instead, we have it as a separate variable that uses `''` (single quote)
      # to avoid expanding, and use it later within the new bash term title. This way both old and
      # new bash see tilde as tilde, and tilde only.
      new_home='~'
      bash_term_tab_title="${PWD/#$HOME/$new_home}"

      if [[ $WARP_IS_LOCAL_SHELL_SESSION == "1" ]]; then
        warp_title "$bash_term_tab_title"
      else
        bash_term_tab_title_remote="${HOSTNAME%%.*}:$bash_term_tab_title"
        warp_title "$bash_term_tab_title_remote"
      fi
    }

    # Runs before executing the command
    warp_set_title_active_on_preexec () {
      # If the user wants to set the title themselves, they can set the WARP_DISABLE_AUTO_TITLE flag.
      if [ ! -z "$WARP_DISABLE_AUTO_TITLE" ]; then
        return
      fi

      cmd="$1"
      # warp_set_title_active_on_preexec is a preexec_function, which accepts 1 argument 
      #(currently invoked command)
      local this_command_spec
      read -r -a this_command_spec <<< "$1"

      # if running fg, figure out the command from its process id
      if [[ "${this_command_spec[0]}" == "fg" ]]; then 
        # note that `fg` and `jobs` essentially take the same arguments (but to be extra safe, 
        # we swapped empty/no-arguments to current background process), so we can just use that
        jobspec=${this_command_spec[1]}
        if [[ "$jobspec" == "" ]]; then
          jobspec="%%"
        fi
        # the output is of format:
        # [1]+  Stopped                 <Command>
        # the following modifications replace all whitespaces to a single space, and remote first
        # two columns from the output (leaving the command name, which itself can include spaces).
        fg_command_name=$(jobs "$jobspec" 2> /dev/null | command -p tr -s ' ' | command -p cut -d ' ' -f3-)
        if [[ "$fg_command_name" == "" ]]; then
          # in the weird case when this returns an empty string, lets just stick to whatever the
          # tab title was originally set to.
          return
        fi
        cmd="$fg_command_name"
      fi

      warp_title "$cmd"
    }

    # The git prompt's git commands are read-only and should not interfere with
    # other processes. This environment variable is equivalent to running with `git
    # --no-optional-locks`, but falls back gracefully for older versions of git.
    # See git(1) for and git-status(1) for a description of that flag.
    #
    # We wrap in a local function instead of exporting the variable directly in
    # order to avoid interfering with manually-run git commands by the user.
    warp_git () {
      GIT_OPTIONAL_LOCKS=0 command git "$@"
    }

    # Note that this is very performance sensitive code, so try not to
    # invoke any external commands in here.
    warp_precmd () {
        # $? is relative to the process so we MUST check this first
        # or else the exit code will correspond to the commands
        # executed within this block instead of the actual last
        # command that was run.
        local exit_code=$?
        if [ "$WARP_IN_MSYS2" = true ]; then
          warp_send_hook_via_kv_pairs_start "CommandFinished"
          warp_send_hook_kv_pair "exit_code" "$exit_code"
          warp_send_hook_kv_pair "next_block_id" "precmd-$WARP_SESSION_ID-$((block_id++))"
          warp_send_hook_via_kv_pairs_end
        else
          warp_send_json_message "{\"hook\": \"CommandFinished\", \"value\": {\"exit_code\": $exit_code, \"next_block_id\": \"precmd-$WARP_SESSION_ID-$((block_id++))\"}}"
        fi

        warp_maybe_send_reset_grid_osc

        if [[ $PS1 == "" ]]; then
          # Use the saved PS1, if we've already unset it (due to active Warp prompt).
          WARP_PS1="$SAVED_PS1"
        else
          # If we haven't unset it yet, then we can use the current PS1 value.
          WARP_PS1="$PS1"
        fi

        # If this is being called for a generator command, short circuit and send an unpopulated
        # precmd payload (except for pwd), since we don't re-render the prompt after generator commands
        # are run.
        if [ ! -z  $_WARP_GENERATOR_COMMAND ]; then
            # Restore the user's precmd_functions, since they were un-registered prior to executing
            # the generator.
            precmd_functions=(${_USER_PRECMD_FUNCTIONS[@]})

            unset _WARP_GENERATOR_COMMAND
            warp_send_json_message "{\"hook\": \"Precmd\", \"value\": {
            \"pwd\": \"\",
            \"ps1\": \"\",
            \"git_head\": \"\",
            \"git_branch\": \"\",
            \"virtual_env\": \"\",
            \"conda_env\": \"\",
            \"node_version\": \"\",
            \"session_id\": $WARP_SESSION_ID,
            \"is_after_in_band_command\": true
            }}"
            return 0
        fi

        # If the files for tracking generator PIDs exist, clear them.
        if [[ -n $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE && -f $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE ]]; then
          echo "" > $_WARP_GENERATOR_PIDS_STARTED_TMP_FILE
        fi
        if [[ -n $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE && -f $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE ]]; then
          echo "" > $_WARP_GENERATOR_PIDS_COMPLETED_TMP_FILE
        fi

        if [[ -z $WARP_INPUT_REPORTING_SUPPORTED ]]; then
          WARP_INPUT_REPORTING_SUPPORTED=$(warp_input_reporting_supported)
        fi

        # If we haven't already, cache information about supported features.
        if [[ -z $WARP_PS1_EXPANSION_SUPPORTED ]]; then
          WARP_PS1_EXPANSION_SUPPORTED=$(warp_ps1_expanding_supported)
        fi

        if [[ $WARP_PS1_EXPANSION_SUPPORTED  == "1" ]]; then
          # When evaluating the PS1, we want to ensure that it's aware of the last exit code.
          # Since we captured it already and executed multiple other commands, the actual
          # last exit code has changed. So before the evaluation, we want to trick the shell
          # into returning the correct value for the $? that may be in PS1
          exit_code_hack() {
            return $1
          }
          exit_code_hack $exit_code
          deref_ps1=${WARP_PS1@P}
        else
          # Tricking the shell into rendering the prompt
          # Note that in more modern versions of bash we could use ${PS1@P} to achieve the same,
          # but MacOS comes by default with a much older version of bash, and we want to be compatible.
          deref_ps1=$(echo -e "\n" | PS1="$WARP_PS1" BASH_SILENCE_DEPRECATION_WARNING=1 "$BASH" --norc -i 2>&1 | command -p head -2 | command -p tail -1)
        fi

        # Escaped PS1 variable
        local escaped_ps1
        if [ "$WARP_IN_MSYS2" = false ]; then
          escaped_ps1=$(warp_escape_ps1 "$(echo "$deref_ps1")")
        fi

        # Flush history
        history -a

        # Reset the custom kill-whole-line binding as the user's bashrc (which is sourced after bashrc_warp)
        # could have added another bind. This won't have any user-impact because these shortcuts are only run
        # in the context of the bash editor, which isn't displayed in Warp.
        bind -r '"\C-p"'
        bind "\C-p":kill-whole-line

        # Reset the report-input binding in case the user's bashrc modified it.
        # This is arbitrarily bound to ESC-i in all supported shells ("i" for input).
        if [[ $WARP_INPUT_REPORTING_SUPPORTED == "1" ]]; then
          bind -r '"\ei"'
          bind -x '"\ei":"warp_report_input"'
        fi
        
        # We need to register bindkeys to enable intra-session switching of the prompt 
        # (these bindkeys are used by Warp to communicate the prompt mode switch to bash).
        # We remove any existing bindkey for ESC-P ("p" for prompt/PS1) and register the bindkey
        # to our custom function. Note that this specific keybinding is arbitrary.
        bind -r '"\ep"'
        bind -x '"\ep":"warp_change_prompt_modes_to_ps1"'
        # We remove any existing bindkey for ESC-P ("w" for Warp prompt) and register the bindkey
        # to our custom function. Note that this specific keybinding is arbitrary.
        bind -r '"\ew"'
        bind -x '"\ew":"warp_change_prompt_modes_to_warp_prompt"'

        local escaped_pwd
        if [ "$WARP_IN_MSYS2" = false ]; then
          if [ -n "$WSL_DISTRO_NAME" ]; then
            # In WSL, avoid symlinks b/c on Windows `std::fs` is unable to resolve symlink inside WSL containers.
            escaped_pwd=$(warp_escape_json "$(pwd -P)")
          else
            escaped_pwd=$(warp_escape_json "$PWD")
          fi
        fi

        local escaped_virtual_env=""
        local escaped_conda_env=""
        local escaped_node_version=""
        local escaped_git_head=""
        local escaped_git_branch=""
        local git_head=""
        local git_branch=""

        # Only fill these fields once we've finished bootstrapping, as the
        # blocks created during the bootstrap process don't have visible
        # prompts, and we don't want to invoke `git` before we've sourced the
        # user's rcfiles and have a fully-populated PATH.
        if [[ -n "$WARP_BOOTSTRAPPED" ]]; then
          if [[ -n "$VIRTUAL_ENV" ]] && [ "$WARP_IN_MSYS2" = false ]; then
              escaped_virtual_env=$(warp_escape_json "$VIRTUAL_ENV")
          fi

          if [[ -n "$CONDA_DEFAULT_ENV" ]] && [ "$WARP_IN_MSYS2" = false ]; then
              escaped_conda_env=$(warp_escape_json "$CONDA_DEFAULT_ENV")
          fi

          # Get Node.js version if node is available and we're in a Node.js project
          if command -v node > /dev/null 2>&1 && [ "$WARP_IN_MSYS2" = false ]; then
              # Check for package.json in current directory and parent directories
              local current_dir="$PWD"
              local found_package_json=false
              local package_json_dir=""
              while [[ "$current_dir" != "/" ]]; do
                  if [[ -f "$current_dir/package.json" ]]; then
                      found_package_json=true
                      package_json_dir="$current_dir"
                      break
                  fi
                  current_dir=$(dirname "$current_dir")
              done
              
              # Only show node version if package.json is within a git repository
              if [[ "$found_package_json" = true ]]; then
                  local git_dir="$package_json_dir"
                  local in_git_repo=false
                  while [[ "$git_dir" != "/" ]]; do
                      if [[ -d "$git_dir/.git" ]]; then
                          in_git_repo=true
                          break
                      fi
                      git_dir=$(dirname "$git_dir")
                  done
                  
                  if [[ "$in_git_repo" = true ]]; then
                      local node_version=$(node --version 2>/dev/null)
                      if [[ -n "$node_version" ]]; then
                          escaped_node_version=$(warp_escape_json "$node_version")
                      fi
                  fi
              fi
          fi

          # Note: We explicitly do _not_ use command -p here, as `git` is a command that can be
          # installed in non-standard locations and so is not always available on the shell's
          # default PATH. Instead, we rely on the active PATH, as if the user doesn't have git
          # available to their session, it is unlikely they will be looking for git branch
          # information from the prompt.
          if command -v git >/dev/null 2>&1; then
            git_branch=$(warp_git symbolic-ref --short HEAD 2> /dev/null)
            # The git branch the user is on, or the git commit hash if they're not on a branch.
            git_head="${git_branch:-$(warp_git rev-parse --short HEAD 2> /dev/null)}"
          fi
          if [ "$WARP_IN_MSYS2" = false ]; then
            escaped_git_head=$(warp_escape_json "$git_head")
            escaped_git_branch=$(warp_escape_json "$git_branch")
          fi
        fi

        # At this point, escaped prompt looks something like
        # \\u{001B}\\u{005B}\\u{0030}\\u{0031}\\u{003B} ...

        # We need to maintain the double quoting of \\u in the message that
        # is sent otherwise the receiving side will interpret the value
        # as JS string literals of the form \uHEX, and will include
        # ctrl characters (like ESC) in the json, which will cause a JSON
        # parse error.
        # Note WARP_SESSION_ID doesn't need to be escaped since it's a number
        # We also pass the shell's notion of `honor_ps1` to ensure it's synced correctly on the Warp-side for prompt handling.
        # This is passed as a "real boolean" via the JSON payload (string interpolated into JSON string below).
        local honor_ps1
        if [[ "$WARP_HONOR_PS1" == "1" ]]; then
          honor_ps1="true"
          # The Warp prompt preview can be rendered using the active prompt in this case (which uses prompt markers).
          escaped_ps1=""
          deref_ps1=""
        else
          honor_ps1="false"
        fi
        # We send the escaped PS1, if we are in active Warp prompt mode, for prompt preview rendering (note the shell's PS1 is unset in this case).
        if [ "$WARP_IN_MSYS2" = true ]; then
          warp_send_hook_via_kv_pairs_start "Precmd"
          warp_send_hook_kv_pair "pwd" "$PWD"
          warp_send_hook_kv_pair_escaped "ps1" "$deref_ps1"
          warp_send_hook_kv_pair "ps1_is_encoded" "false"
          warp_send_hook_kv_pair "honor_ps1" "$honor_ps1"
          warp_send_hook_kv_pair "git_head" "$git_head"
          warp_send_hook_kv_pair "git_branch" "$git_branch"
          warp_send_hook_kv_pair "virtual_env" "$VIRTUAL_ENV"
          warp_send_hook_kv_pair "conda_env" "$CONDA_DEFAULT_ENV"
          warp_send_hook_kv_pair "node_version" "$node_version"
          warp_send_hook_kv_pair "session_id" "$WARP_SESSION_ID"
          warp_send_hook_via_kv_pairs_end
        else
          local escaped_json="{\"hook\": \"Precmd\", \"value\": {
          \"pwd\": \"$escaped_pwd\",
          \"ps1\": \"$escaped_ps1\",
          \"honor_ps1\": $honor_ps1,
          \"ps1_is_encoded\": true,
          \"git_head\": \"$escaped_git_head\",
          \"git_branch\": \"$escaped_git_branch\",
          \"virtual_env\": \"$escaped_virtual_env\",
          \"conda_env\": \"$escaped_conda_env\",
          \"node_version\": \"$escaped_node_version\",
          \"session_id\": $WARP_SESSION_ID
          }}"
          warp_send_json_message "$escaped_json"
        fi
    }

    warp_clear_on_next_block () {
        warp_send_json_message '{"hook": "ClearOnNextBlock"}'
    }

    # Format a string value according to JSON syntax.
    warp_escape_json () {
        # Explanation of the sed replacements (each command is separated by a `;`):
        # s/(["\\])/\\\1/g - Replace all double-quote (") and backslash (\) characters with the escaped versions (\" and \\)
        # s/\b/\\b/g - Replace all backspace characters with \b
        # s/\t/\\t/g - Replace all tab characters with \t
        # s/\f/\\f/g - Replace all form feed characters with \f
        # s/\r/\\r/g - Replace all carriage return characters with \r
        # $!s/$/\\n/ - On every line except the last, insert the \n escape at the end of the line
        #              Note: sed acts line-by-line, so it doesn't see the literal newline characters to replace
        #
        # tr -d '\n' - Remove the literal newlines from the final output
        #
        # Additional note: In a shell script between single quotes ('), no escape sequences are interpreted.
        # To work around that and insert the literal values into the regular expressions, we stop the single-quote,
        # then add the literal using ANSI-C syntax ($'\t'), then start a new single-quote. That is the meaning
        # behind the various `'$'\b''` blocks in the command. All of these separate strings are then concatenated
        # together to form the full argument to send to sed.
        #
        # We also use 'command sed' to ensure that we aren't accidentally executing an alias or function named 'sed'
        command -p sed -E 's/(["\\])/\\\1/g; s/'$'\b''/\\b/g; s/'$'\t''/\\t/g; s/'$'\f''/\\f/g; s/'$'\r''/\\r/g; $!s/$/\\n/' <<<"$*" | command -p tr -d '\n'
    }

    # Turns out that the processed prompt is a complicated data structure that includes lots of 
    # information that's passed to the shell (including the actual shell, version, working directory
    # and, well, the prompt itself). What is more, prompt can also include emojis - unicode characters
    # that sometimes contain special bytes (ie. ST, CAN or SUB) that are otherwise used as unhook
    # triggers for the precmd. Instead of escaping those and extracting the value of the prompt itself,
    # we simply convert the entire data structure into a single line hex string, which Warp
    # later decodes and sends to the grid to show the prompt.
    # Note: before converting the prompt to a hex string, we remove the multi-line newlines and replace
    # them with a single space (to avoid prompts that span multiple empty lines).
    warp_escape_ps1 () {
       command -p tr '\n\n' ' ' <<< "$*" | command -p od -An -v -tx1 | command -p tr -d ' \n'
    }

    # warp_hex_encode_string encodes the entire DCS string (JSON) with od making it essentially
    # a very long hexadecimal string.
    # Afterwards it's decoded in rust and parsed as usual.
    # Accepts one argument: DCS JSON string
    warp_hex_encode_string () {
      echo "$1" | command -p od -An -v -tx1 | command -p tr -d ' \n'
    }

    # Returns encoded InitShell hook
    # Accepts one argument: shell [bash, zsh, fish (future)]
    init_shell_hook () {
      init_shell="{\"hook\": \"InitShell\", \"value\": {\"shell\": \"$1\"}}"
      echo $(warp_hex_encode_string "$init_shell")
    }

    # Checks whether the current version of bash is at least as high as the expected ($1) one.
    # To match rest of our codebase, it returns "1" if the bash version is higher or equal, and 
    # 0 otherwise.
    warp_at_least_bash_version () {
      if [[ $(printf '%s\n%s\n' "$BASH_VERSION" "$1" | command -p sort -rVC ; echo $?) -eq 0 ]]; then
        echo "1"
      else 
        echo "0"
      fi
    }

    # @P substitution was introduced in 4.4 bash version, so it returns "1" if the current bash
    # version is 4.4 or higher.
    warp_ps1_expanding_supported () {
      warp_at_least_bash_version "4.4"
    }

    # The $READLINE_LINE variable in `bind -x` sequences was introduced in bash 4.0,
    # so we can only report the input buffer if the bash version is 4.0 or higher.
    warp_input_reporting_supported () {
        warp_at_least_bash_version "4.0"
    }

    # Report the current input buffer contents to Warp. This only works correctly
    # if `warp_input_reporting_supported` returns "1".
    warp_report_input () {
        if [ "$WARP_IN_MSYS2" = true ]; then
            warp_send_hook_via_kv_pairs_start "InputBuffer"
            warp_send_hook_kv_pair "buffer" "$READLINE_LINE"
            warp_send_hook_via_kv_pairs_end
        else
            local escaped_input="$(warp_escape_json "$READLINE_LINE")"
            warp_send_json_message "{ \"hook\": \"InputBuffer\", \"value\": { \"buffer\": \"$escaped_input\" } }"
        fi
        # This prevents bash from re-printing typeahead after we've removed it.
        READLINE_LINE=""
    }

    # Check whether the prompt-related variables have OSC prompt marker sequences,
    # and if not, wrap them with the appropriate markers so that we can direct the
    # prompt bytes to the appropriate grids.
    function warp_update_prompt_vars() {
      # 133;A and 133;B are standard prompt marker OSCs.
      # See https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration and
      # https://gitlab.freedesktop.org/terminal-wg/specifications/-/merge_requests/6/diffs for details.
      local prompt_prefix=$'\e]133;A\a'
      local prompt_suffix=$'\e]133;B\a'
      if [[ "$WARP_HONOR_PS1" != "1" ]] && [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then
        local suffix="$prompt_suffix$RESET_GRID_OSC"
      else
        local suffix="$prompt_suffix"
      fi

      # The "\[" and "\]" indicate to bash that the sequence between the markers are 
      # "non-printable", which effectively means they should not change the position
      # of the cursor as they're "printed".
      local prompt_prefix_with_cursor_marker_surrounded="\[$prompt_prefix\]"
      local suffix_with_cursor_marker_surrounded="\[$suffix\]"

      # Clear the user-defined prompt again, if using Warp's built-in prompt, before the command 
      # is rendered as it could have been reset by the user's bashrc or by setting 
      # the variable on the command line. This is used for same-line prompt and leads to the temporary
      # product behavior of Warp prompt switches only taking effect in new sessions.
      # Certain prompt plugins can reset the prompt to a non-empty value, after we've initially unset it.
      # Confirm that it is unset, if using built-in Warp prompt (update prompt vars is forced to run as the last precmd fn).
      if [[ "$WARP_HONOR_PS1" != "1" ]]; then
        if [[ "$PS1" != "" ]]; then
          # If the PS1 has its original value, then we save it in SAVED_PS1 so we can restore to this value, if we were to unset it for
          # the Warp prompt case, but the user wants to switch back to PS1 later.
          SAVED_PS1=$PS1
        fi
        # Note that we DO NOT unset the PS1 here, since we want to pass it along as a "hidden left prompt" for 
        # prompt preview purposes, if the Warp prompt is being used. Specifically, we want to show this prompt preview
        # for the Edit Prompt modal and onboarding prompt block.
      fi

      if [[ -n "$PS1" ]]; then
        # Remove any existing prompt/cursor markers from the prompt, before we re-modify it. Notably,
        # we want to wrap the latest version of the prompt (which may have been changed e.g. virtualenv
        # added).
        if [[ "$PS1" == *"$prompt_prefix_with_cursor_marker_surrounded"* ]]; then
          local preceding_prefix=${PS1%%"$prompt_prefix_with_cursor_marker_surrounded"*}
          local following_prefix=${PS1#*"$prompt_prefix_with_cursor_marker_surrounded"}
          PS1=$preceding_prefix$following_prefix
        fi
        if [[ "$PS1" == *"$suffix_with_cursor_marker_surrounded"* ]]; then
          local preceding_suffix=${PS1%"$suffix_with_cursor_marker_surrounded"*}
          local following_suffix=${PS1##*"$suffix_with_cursor_marker_surrounded"}
          PS1=$preceding_suffix$following_suffix
        fi

        ORIGINAL_PS1=$PS1
        PS1="$prompt_prefix$PS1$suffix"
      fi

      # Unset the PS1, if we are using the Warp prompt.
      if [[ "$WARP_HONOR_PS1" != "1" ]]; then
        PS1=""
      # Otherwise, if we are using the PS1, we use the normal prompt markers.
      else
        if [[ "$PS1" != "\["*"\]" ]]; then
          # We surround the non-printable OSCs with cursor markers to make sure the shell does NOT
          # account for them when keeping track of its internal cursor position.
          PS1="\[$prompt_prefix\]$ORIGINAL_PS1\[$suffix\]"
        fi
      fi

      # Ensure that this is always the last precmd hook. This prevents any other precmd hook, which might
      # modify $PS1, from interfering with our prompt-escaping logic.
      #
      # Remove warp_update_prompt_vars from the precmd_functions list and then re-append it to ensure it's
      # ordered last.

      # Initialize an empty array to hold the filtered functions.
      filtered_precmd_functions=()

      # Loop through each function in the original precmd_functions array.
      for func in "${precmd_functions[@]}"; do
        # Add the function to the filtered array if it's not warp_update_prompt_vars
        if [[ "$func" != "warp_update_prompt_vars" ]]; then
          filtered_precmd_functions+=("$func")
        fi
      done

      # Assign the filtered array back to precmd_functions.
      precmd_functions=("${filtered_precmd_functions[@]}")

      # Append warp_update_prompt_vars to the end of the precmd_functions array.
      precmd_functions+=("warp_update_prompt_vars")
    }
    
    # Changes the WARP_HONOR_PS1 variable to 1, to indicate we want to use the PS1. Restores
    # the original PS1 value (which we unset for Warp prompt) and calls warp_update_prompt_vars
    # to refresh the prompt. Note that we use an "empty block" workaround to achieve instant
    # prompt switching in bash, since there is no built-in methods to repaint the prompt, unlike
    # Zsh/fish.
    function warp_change_prompt_modes_to_ps1() {
      PS1="$SAVED_PS1"
      WARP_HONOR_PS1="1"

      warp_update_prompt_vars
    }

    # Changes the WARP_HONOR_PS1 variable to 0, to indicate we want to use the Warp prompt. Calls 
    # warp_update_prompt_vars to refresh the prompt (note the PS1 will be unset in this logic). 
    # Note that we use an "empty block" workaround to achieve instant prompt switching in bash, 
    # since there is no built-in methods to repaint the prompt, unlike Zsh/fish.
    function warp_change_prompt_modes_to_warp_prompt() {
      WARP_HONOR_PS1="0"

      warp_update_prompt_vars
    }

    function clear() {
        if [ "$WARP_IN_MSYS2" = true ]; then
            warp_send_hook_via_kv_pairs_start "Clear"
            warp_send_hook_via_kv_pairs_end
        else
            warp_send_json_message "{\"hook\": \"Clear\", \"value\": {}}"
        fi
    }

    function warp_finish_update {
      local update_id="$1"
      if [ "$WARP_IN_MSYS2" = true ]; then
        warp_send_hook_via_kv_pairs_start "FinishUpdate"
        warp_send_hook_kv_pair "update_id" "$update_id"
        warp_send_hook_via_kv_pairs_end
      else
        warp_send_json_message "{ \"hook\": \"FinishUpdate\", \"value\": { \"update_id\": \"$update_id\"} }"
      fi
    }

    # Check if the warp apt source file has been renamed to `warpdotdev.list.distUpgrade` due to an ubuntu version update.
    # If this occurred, we want to rename the source file back to `warpdotdev.list` to ensure updates can proceed.
    # We purposefully skip this if either the `warpdotdev.list` file already exists (indicating that the user has already
    # done this themselves) _or_ if a `warpdotdev.sources` file exists (which is the new Deb822 format for source files).
    # The `.sources` file could only exist if a user manually created it; Ubuntu doesn't create one automatically for the
    # warp source file due to a bug in its update flow where it considers our source file to be "invalid" because it
    # contains a `signed-by` key.
    function warp_handle_dist_upgrade {
      local source_file_name="$1"

      eval "$(command apt-config shell APT_SOURCESDIR 'Dir::Etc::sourceparts/d')"

      if [[ ! -e $APT_SOURCESDIR$source_file_name.list && \
          ! -e $APT_SOURCESDIR$source_file_name.sources && \
           -e $APT_SOURCESDIR$source_file_name.list.distUpgrade ]]; then
        # DO NOT DO THIS. We should never run a command for user with `sudo`. The only reason this is safe here is because
        # we insert this function into the input for the user to determine if they want to execute (we never run it on
        # their behalf without their permission).  To be transparent about what is being executed with sudo, we echo out the
        # command we're about to run.
        echo "Executing: sudo cp \"$APT_SOURCESDIR$source_file_name.list.distUpgrade\" \"$APT_SOURCESDIR$source_file_name.list\""
        sudo cp "$APT_SOURCESDIR$source_file_name.list.distUpgrade" "$APT_SOURCESDIR$source_file_name.list"
      fi
    }

    # The SSH logic only applies to local sessions, because we don't yet have support for bootstrapping
    # recursive SSH sessions.
    if [[ $WARP_IS_LOCAL_SHELL_SESSION == "1" ]]; then
        # This helper function determines whether the user's ssh arguments imply
        # creation of a non-interactive session or otherwise would conflict with
        # our SSH wrapper.  Returns 0 for an interactive session; >0 otherwise.
        function is_interactive_ssh_session() {
            ARGS=()    # this array holds any positional arguments
            while [ $# -gt 0 ]; do
                # Initialize this to 1 before each call, as per getopts documentation.
                OPTIND=1
                # Parse through all ssh options, as defined in the ssh man pages.
                while getopts :1246AaCfgKkMNnqsTtVvXxYyb:c:D:e:F:i:L:l:m:O:o:p:R:S:W:w: OPTION; do
                    case $OPTION in
                        # -T disables pty allocation (aka a non-interactive session)
                        T) return 1;;
                        # -W implies -T
                        W) return 1;;
                        # The user provided an invalid option; kick it to real ssh.
                        \?) return 1;;
                        # The user omitted a required argument; kick it to real ssh.
                        :) return 1;;
                    esac
                done
                [ $? -eq 0 ] || return 2       # getopts failed
                [ $OPTIND -gt $# ] && break    # we reached the end of the parameters

                shift "$((OPTIND - 1))"  # skip all options processed so far
                ARGS[${#ARGS[@]}]=$1     # save first non-option argument (a.k.a. positional argument)
                shift                    # remove saved arg
            done

            # If there is more than one positional argument, the user is attempting to
            # run a command, not start an interactive session.  If there is less than
            # one positional argument, the user should be shown the usage text.
            if [[ ${#ARGS[@]} -ne 1 ]]; then
                return 1
            fi
        }

        function warp_ssh_helper() {
            init_shell_bash=$(init_shell_hook "bash")
            init_shell_zsh=$(init_shell_hook "zsh")

            # Hex-encode the ZSH environment script we use to bootstrap remote zsh b/c it contains control characters
            # We decode on the SSH server using xxd if its available, otherwise fall back to a for-loop over each byte
            # and use printf to convert back to plaintext
            local zsh_env_script=$(printf '%s' 'unsetopt ZLE; unset RCS; unset GLOBAL_RCS; WARP_SESSION_ID="$(command -p date +%s)$RANDOM"; WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@; WARP_HONOR_PS1='$WARP_HONOR_PS1'; _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n); _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER); _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"zsh\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d '"'"' \n'"'"'); printf '"'"'\e]9278;d;%s\x07'"'"' $_msg; unset _hostname _user _msg' | command -p od -An -v -tx1 | command -p tr -d ' \n')

            # Keep remote commands up-to-date with shell.rs & bash.sh.
            # Note that in this command, we're passing a string to the remote shell. Any variable expansions need to be
            # escaped with "''" to avoid the local shell from expanding them before they're passed to the remote shell.
            # We check the SHELL env var and use shell string manipulation to get the contents after the last slash to
            # determine what shell is the login shell on the remote machine.  We perform a preliminary check to see if
            # the remote shell is the Bourne shell to avoid asking it to parse later lines that use syntax it doesn't
            # support.
            command ssh -o ControlMaster=yes -o ControlPath=$SSH_SOCKET_DIR/$WARP_SESSION_ID \
            -t "${@:1}" \
"
export TERM_PROGRAM='WarpTerminal'
# Mark the remote side of a Warp-managed SSH session so the bootstrap
# body can distinguish it from local shells. Used to gate the ExitShell
# hook which tears down the remote-server-proxy subprocess.
export WARP_IS_SSH='1'
test -n '$WARP_CLIENT_VERSION' && export WARP_CLIENT_VERSION='$WARP_CLIENT_VERSION'
# Only forward the protocol version if it was set locally (i.e. the HOANotifications feature flag is on).
test -n '$WARP_CLI_AGENT_PROTOCOL_VERSION' && export WARP_CLI_AGENT_PROTOCOL_VERSION='$WARP_CLI_AGENT_PROTOCOL_VERSION'

hook="'$(printf "{\"hook\": \"SSH\", \"value\": {\"socket_path\": \"'$SSH_SOCKET_DIR/$WARP_SESSION_ID'\", \"remote_shell\": \"%s\"}}" "${SHELL##*/}" | command -p od -An -v -tx1 | command -p tr -d " \n")'"
printf '$OSC_START$DCS_JSON_MARKER$OSC_PARAM_SEPARATOR%s$OSC_END' "'$hook'"

if test "'"${SHELL##*/}" != "bash" -a "${SHELL##*/}" != "zsh"'"; then
  # Emulate the SSHD logic to print the MotD. Because the Warp SSH wrapper passes
  # a command to run, SSHD does a quiet login, updating utmp and other login
  # state, but not printing the MotD. For bash and zsh, this is instead handled
  # by our bootstrap script.
  if test ! -e "'$HOME/.hushlogin'"; then
    # This uses an if-else chain instead of a for-loop to avoid expansion issues on older shells.
    if test -r /etc/motd; then
      command -p cat /etc/motd
    elif test -r /run/motd; then
      command -p cat /run/motd
    elif test -r /run/motd.dynamic; then
      command -p cat /run/motd.dynamic
    elif test -r /usr/lib/motd; then
      command -p cat /usr/lib/motd
    elif test -r /usr/lib/motd.dynamic; then
      command -p cat /usr/lib/motd.dynamic
    fi
  fi
  # Likewise, emulate a login shell by sourcing /etc/profile
  if test -r /etc/profile; then
    . /etc/profile
  fi
  exec "'$SHELL'"
fi

case "'${SHELL##*/}'" in
  bash)
    exec -a bash bash --rcfile <(echo '"'
      command -p stty raw
      HISTCONTROL=ignorespace
      HISTIGNORE=" *"
      WARP_SESSION_ID="$(command -p date +%s)$RANDOM"
      WARP_HONOR_PS1="'$WARP_HONOR_PS1'"
      _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n)
      _user=$(command -v whoami >/dev/null 2>&1 && command whoami 2>/dev/null || echo $USER)
      _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"bash\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n")'"
      WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@
      if [[ "'$OS'" == Windows_NT ]]; then WARP_IN_MSYS2=true; else WARP_IN_MSYS2=false; fi
      printf '\''"'\e]9278;d;%s\x07'"'\'' \""'$_msg'"\"')
      unset _hostname _user _msg
      ;;
  zsh) WARP_TMP_DIR="'$(command -p mktemp -d warptmp.XXXXXX)'"
local ZSH_ENV_SCRIPT='$zsh_env_script'
if [[ "'$?'" == 0 ]]; then
  if command -pv xxd >/dev/null 2>&1; then
    echo "'$ZSH_ENV_SCRIPT'" | command -p xxd -p -r > "'$WARP_TMP_DIR'"/.zshenv
  else
    for i in {0..\$((\${#ZSH_ENV_SCRIPT} - 1))..2}; do
      builtin printf "'"\x${ZSH_ENV_SCRIPT:$i:2}"'"
    done > "'$WARP_TMP_DIR'"/.zshenv
  fi
else
  echo \"Failed to bootstrap warp. Continuing with a non-bootstrapped shell.\"
fi
TMPPREFIX="'$HOME/.zshtmp-'" WARP_SSH_RCFILES="'${ZDOTDIR:-$HOME}'" ZDOTDIR="'$WARP_TMP_DIR'" exec -l zsh -g $TRACE_FLAG_IF_WARP_SHELL_DEBUG_MODE
      ;;
esac
"
        }

        function ssh() {
            if is_interactive_ssh_session "$@"; then
                warp_send_json_message "{\"hook\": \"PreInteractiveSSHSession\", \"value\": {}}"

                # If the SSH wrapper is not enabled for this session, don't use it.
                if [ "$WARP_USE_SSH_WRAPPER" = "1" ]; then
                    local TRACE_FLAG_IF_WARP_SHELL_DEBUG_MODE=""
                    if [[ "$WARP_SHELL_DEBUG_MODE" == "1" ]]; then
                        TRACE_FLAG_IF_WARP_SHELL_DEBUG_MODE="-x"
                    fi
                    warp_ssh_helper "$@"
                else
                    command ssh "$@"
                fi
            else
                command ssh "$@"
            fi
        }
    fi


    # Send a precmd message to the terminal to differentiate between the warp
    # bootstrap logic pasted into the PTY and the output of shell startup files.
    warp_precmd

    # Before calling rcfiles, print the MotD.
    # In general, login(1) or pam_motd(8) is supposed to do this. However, we don't
    # go through the normal login flow when bootstrapping a Warp session. In
    # addition, Warp bash shells are _not_ login shells, because bash does not
    # support custom rcfiles in login shells.
    if [[ ! -e "$HOME/.hushlogin" ]]; then
      for motd_file in /etc/motd /run/motd /run/motd.dynamic /usr/lib/motd /usr/lib/motd.dynamic; do
        if [[ -r "$motd_file" ]]; then
          command -p cat "$motd_file"
          break
        fi
      done
    fi


    # This reflects the bootstrap sequence in a login shell. We want to
    # Do other shell startup first so we can ensure Warp goes last.
    #
    # If this is a subshell, the user and system RC files have already been sourced.
    if [[ -z $WARP_IS_SUBSHELL ]]; then
        # Make sure we force the locale used for number formatting to "C", to avoid
        # issues in locales that use a comma as the decimal separator.
        rcfiles_start_time="$(LC_ALL="C"; echo $EPOCHREALTIME)"

        if [[ -e /etc/profile ]]; then
            source /etc/profile;
        elif [[ -e /etc/bash.bashrc ]]; then
            # /etc/bash.bashrc is only included in the startup sequence if bash is
            # compiled with -DSYS_BASHRC. This is disabled in bash by default but
            # included in Debian and adopted in other distributions. There's no efficient way
            # to tell if it's enabled or not, so just check for the file if there is
            # no /etc/profile and source it if it exists.
            # Note that a lot of the time, /etc/profile sources /etc/bash.bashrc.
            source /etc/bash.bashrc
        fi

        if [[ -e $HOME/.bash_profile ]]; then
            source $HOME/.bash_profile
        elif [[ -e $HOME/.bash_login ]]; then
            source $HOME/.bash_login
        elif [[ -e $HOME/.profile ]]; then
            source $HOME/.profile
        fi

        rcfiles_end_time="$(LC_ALL="C"; echo $EPOCHREALTIME)"
    fi

    # Unset HISTFILESIZE and HISTSIZE if the user rcfiles didn't change them
    # away from our very large sentinel values.  We need to set the initial
    # values to ensure that the user's history file and in-memory history list
    # don't get truncated when we spawn the shell, but once bootstrap has
    # completed, we want the values to be what they would have been if we hadn't
    # set initial values.
    #
    # For more context, see: https://github.com/warpdotdev/Warp/issues/1262
    if [[ $HISTFILESIZE == $WARP_INITIAL_HISTFILESIZE ]]; then
        unset HISTFILESIZE
    fi
    unset WARP_INITIAL_HISTFILESIZE
    if [[ $HISTSIZE == $WARP_INITIAL_HISTSIZE ]]; then
        unset HISTSIZE
    fi
    unset WARP_INITIAL_HISTSIZE

    # Save the value of HISTCONTROL as it existed just after reading the user's
    # rcfiles.
    USER_HISTCONTROL="$HISTCONTROL"

    # Add a pattern to ignore in-band commands in shell history, while preserving the user's
    # HISTIGNORE value which may been set in an RC file sourced above. It is important to
    # ensure that this happens _after_ the user's RC files have been sourced.
    if [[ ! -z $HISTIGNORE ]]; then
        HISTIGNORE="*warp_run_generator_command*:$HISTIGNORE"
    else
        HISTIGNORE="*warp_run_generator_command*"
    fi

    # If the user has PROMPT_COMMAND set in their bootstrap scripts,
    # save it into USER_PROMPT_COMMAND and install a precmd hook so we can
    # respect it. It behaves exactly the same as precmd (i.e. executed
    # before the prompt is shown).
    # Since bash-preexec actually uses PROMPT_COMMAND, we shouldn't do this if bash-preexec
    # is installed from /etc/bash.bashrc. If it's included in the user's rcfiles, it
    # should've guarded on the presence of bash_preexec_imported and no-oped.
    if [[ -n $PROMPT_COMMAND && -z $BASH_PREEXEC_IN_ETC_BASHRC ]]; then
        # Do not honor the user's PROMPT_COMMAND if it re-assigns PROMPT_COMMAND, as that
        # will break the precmd hook. This is the case in Kali Linux's default .bashrc
        # https://gist.github.com/Searge/158f9061da831a53eeacff44eac71447#file-bashrc-L100
        if [[ ! $PROMPT_COMMAND =~ "PROMPT_COMMAND=" ]]; then
            # We treat the PROMPT_COMMAND as an array in case users are using bash 5.1
            # array variant of PROMPT_COMMAND. If PROMPT_COMMAND is a string, this will
            # still work, just create a length 1 array.
            USER_PROMPT_COMMAND=("${PROMPT_COMMAND[@]}")
            function user_prompt_command() {
                for command in "${USER_PROMPT_COMMAND[@]}"; do
                    eval $command
                done
            }
        fi
        # We need to unset PROMPT_COMMAND here. Otherwise, it will be executed twice.
        unset PROMPT_COMMAND
    fi

    # If the user's rc files turned PROMPT_COMMAND into an array, we must undo that.
    # Since Bash 5.1, the PROMPT_COMMAND variable can be an array, see:
    #   https://tiswww.case.edu/php/chet/bash/NEWS
    # Unfortunately, doing so will break Warp because of the way it interacts with bash-preexec.
    # When PROMPT_COMMAND is an array, the DEBUG signal fires for each array element, and since
    # bash-preexec uses a DEBUG trap to trigger the preexec functions, it will run our preexec
    # functions before the command at PROMPT_COMMAND[1], PROMPT_COMMAND[2], etc. This means our
    # Preexec hook gets called without the user submitting a command, putting the input block into
    # a broken state, e.g. see https://github.com/warpdotdev/Warp/issues/2636
    # If they end up fixing this, we may be able to remove this at some point, check this:
    #   https://github.com/rcaloras/bash-preexec/issues/130
    #
    # If PROMPT_COMMAND is set and it's an array, "join" the array into a newline-delimited string,
    # then overwrite PROMPT_COMMAND
    if [[ -n $PROMPT_COMMAND && "$(declare -p PROMPT_COMMAND)" =~ "declare -a" ]]; then
        PROMPT_COMMAND_FLATTENED=$(IFS=$'\n'; echo "${PROMPT_COMMAND[*]}")
        unset PROMPT_COMMAND
        PROMPT_COMMAND=$PROMPT_COMMAND_FLATTENED
    fi

## ----- Bash_preexec initialization -----
    # If bash-preexec is already installed, we don't install it again.
    if [[ "${PROMPT_COMMAND:-}" != *"__bp_precmd_invoke_cmd"* ]]; then
        install_bashpreexec
    else
        # If we changed the HISTCONTROL value to work around the behavior in Debian bash,
        # we want to re-run the logic so bash-preexec works properly (otherwise, it won't
        # receive the command in preexec).
        __bp_adjust_histcontrol
    fi
## ----- Warp initialization -----
    
    # Append additional PATH entries if provided via WARP_PATH_APPEND. This is after the user's RC
    # files are sourced in case they reset PATH (/etc/profile on Debian does this, for example).
    if [[ ! -z "$WARP_PATH_APPEND" ]]; then
        export PATH="$PATH:$WARP_PATH_APPEND"
        unset WARP_PATH_APPEND
    fi

    # Read through shell options to determine if the user has enabled vi mode.
    vi_mode_enabled=0
    IFS=':' read -ra SHELLOPT <<< "$SHELLOPTS"
    for i in "${SHELLOPT[@]}"; do
        if [[ "$i" == "vi" ]]; then
            vi_mode_enabled=1
        fi
    done

  if kernel_name="$(uname)"; then
    if [[ "$kernel_name" == "Darwin" ]]; then
      os_category="MacOS"
    elif [[ "$kernel_name" == "Linux" ]]; then
      os_category="Linux"
      default_os_release_filepath="/etc/os-release"
      fallback_os_release_filepath="/usr/lib/os-release"
      # We first try /etc/os-release and then try /usr/lib/os-release as a fallback.
      if test -f "$default_os_release_filepath"; then
        os_release_file="$default_os_release_filepath"
      elif test -f "$fallback_os_release_filepath"; then
        os_release_file="$fallback_os_release_filepath"
      fi
      if test -f "$os_release_file"; then
        linux_distribution="$(cat $os_release_file | sed -nE 's/^NAME="(.*)"$/\1/p')"
      fi
    fi
  fi

    precmd_functions+=(warp_precmd)
    preexec_functions+=(warp_preexec)

    precmd_functions+=(warp_set_title_idle_on_precmd)
    preexec_functions+=(warp_set_title_active_on_preexec)

    if declare -f user_prompt_command 2>&1 >/dev/null; then
        precmd_functions+=(user_prompt_command)
    fi

    WARP_BOOTSTRAPPED=1

    warp_update_prompt_vars

    # Set the history file to append
    shopt -s histappend

    shell_plugins=()

    function warp_bootstrapped () {
        local aliases="`alias`"
        local env_var_names="`compgen -e`"
        local function_names="`compgen -A function`"
        local builtins="`compgen -b`"
        local keywords="`compgen -k`"
        if [ "$WARP_IN_MSYS2" = false ]; then
          # Note that for now we don't support dynamically changing HISTFILE within a session.
          local escaped_histfile="$(warp_escape_json "$HISTFILE")"
          local escaped_abbrs=""
          local escaped_aliases="$(warp_escape_json "$aliases")"
          local escaped_env_var_names="$(warp_escape_json "$env_var_names")"
          local escaped_function_names="$(warp_escape_json "$function_names")"
          local escaped_builtins="$(warp_escape_json "$builtins")"
          local escaped_keywords="$(warp_escape_json "$keywords")"
        fi

        local shell_options="`shopt -s | command -p cut -f 1`"
        # Provide terminal logic access to the value of HISTCONTROL via shell
        # options.  Prefix the fake option with "!" to avoid conflicts with
        # real bash options.
        if [[ -n $USER_HISTCONTROL ]]; then
            shell_options="$shell_options \n !histcontrol_$USER_HISTCONTROL"
        fi

        # Check if Starship is active for Bash. Note that another prompt could still be overriding Starship, however,
        # this is our best guess for the currently active prompt plugin.
        if [ "$STARSHIP_SHELL" = "bash" ]; then
          shell_plugins+=("starship")
        fi

        if [ "$WARP_IN_MSYS2" = false ]; then
          local escaped_shell_plugins=$(warp_escape_json "$shell_plugins")
          local escaped_path="$(warp_escape_json "$PATH")"
          local escaped_shell_options=$(warp_escape_json "$shell_options")
        fi

        local _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER)
        local _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n)
        if [ "$WARP_IN_MSYS2" = true ]; then
          warp_send_hook_via_kv_pairs_start "Bootstrapped"
          warp_send_hook_kv_pair "histfile" "$HISTFILE"
          warp_send_hook_kv_pair "session_id" "$WARP_SESSION_ID"
          warp_send_hook_kv_pair "shell" "bash"
          warp_send_hook_kv_pair "home_dir" "$HOME"
          warp_send_hook_kv_pair "user" "$_user"
          warp_send_hook_kv_pair "hostname" "$_hostname"
          warp_send_hook_kv_pair "path" "$PATH"
          warp_send_hook_kv_pair "cdpath" "$CDPATH"
          warp_send_hook_kv_pair_escaped "env_var_names" "$env_var_names"
          warp_send_hook_kv_pair "abbreviations" ""
          warp_send_hook_kv_pair_escaped "aliases" "$aliases"
          warp_send_hook_kv_pair_escaped "function_names" "$function_names"
          warp_send_hook_kv_pair_escaped "builtins" "$builtins"
          warp_send_hook_kv_pair_escaped "keywords" "$keywords"
          warp_send_hook_kv_pair "shell_plugins" "$shell_plugins"
          warp_send_hook_kv_pair "shell_version" "$BASH_VERSION"
          warp_send_hook_kv_pair "shell_options" "$shell_options"
          warp_send_hook_kv_pair "rcfiles_start_time" "$rcfiles_start_time"
          warp_send_hook_kv_pair "rcfiles_end_time" "$rcfiles_end_time"
          warp_send_hook_kv_pair "vi_mode_enabled" "$vi_mode_enabled"
          warp_send_hook_kv_pair "os_category" "$os_category"
          warp_send_hook_kv_pair "linux_distribution" "$linux_distribution"
          warp_send_hook_kv_pair "wsl_name" "$WSL_DISTRO_NAME"
          warp_send_hook_kv_pair "shell_path" "$BASH"
          warp_send_hook_via_kv_pairs_end
        else
          local escaped_editor="$(warp_escape_json "$EDITOR")"
          local escaped_shell_path="$(warp_escape_json "$BASH")"
          local escaped_cdpath="$(warp_escape_json "$CDPATH")"
          local escaped_json="{\"hook\": \"Bootstrapped\", \"value\": {\"histfile\": \"$escaped_histfile\", \"session_id\": $WARP_SESSION_ID, \"shell\": \"bash\",  \"home_dir\": \"$HOME\", \"user\":\"$_user\", \"host\":\"$_hostname\", \"path\": \"$escaped_path\", \"cdpath\": \"$escaped_cdpath\", \"editor\": \"$escaped_editor\", \"env_var_names\": \"$escaped_env_var_names\", \"abbreviations\": \"$escaped_abbrs\", \"aliases\": \"$escaped_aliases\", \"function_names\": \"$escaped_function_names\", \"builtins\": \"$escaped_builtins\", \"keywords\": \"$escaped_keywords\", \"shell_version\": \"$BASH_VERSION\", \"shell_options\": \"$escaped_shell_options\", \"rcfiles_start_time\": \"$rcfiles_start_time\", \"rcfiles_end_time\": \"$rcfiles_end_time\", \"vi_mode_enabled\": \"$vi_mode_enabled\", \"os_category\": \"$os_category\", \"linux_distribution\": \"$linux_distribution\", \"wsl_name\": \"$WSL_DISTRO_NAME\", \"shell_path\": \"$escaped_shell_path\"}}"
          warp_send_json_message "$escaped_json"
        fi
    }
    warp_bootstrapped
fi
