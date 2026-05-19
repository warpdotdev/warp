# Note that WARP_SESSION_ID is expected to have been set when executing commands to
# emit the InitShell payload, which includes the session ID.
begin
# We wrap ourselves in a begin block because these are effectively injected
# keystrokes, and we want to execute this whole script as a single command.
# Of course we are fish script, not POSIX shell script.
# Set fish_private_mode to prevent saving to history.
# Note: We don't add a leading space to the `begin` command here because this entire script is
# pasted into fish with bracketed paste, and Fish strips out the leading whitespace. Instead,
# we insert the space _before_ pasting the script in, which has the same effect
set -l saved_fish_private_mode $fish_private_mode
set -g fish_private_mode 1

# Disable fish autosuggestions - because input goes through Warp's editor instead,
# they are never actionable, and the extra output can cause problems.
set -g fish_autosuggestion_enabled 0

# Byte sequence used to signal the start of a DCS. ([0x1b, 0x50, 0x24] which
# maps to <ESC>, P, $ in ASCII.)
set -g DCS_START \u1b\u50\u24

# Appended to $DCS_START to signal that the following message is JSON-encoded.
# The Rust app also receives non-JSON-encoded DCS's sent from
# _warp_run_generator_command_internal, which instead end in 'e' (0x65).
set -g DCS_JSON_MARKER 'd'

set -g DCS_END \u9c

set -g OSC_START (printf '\e]9278;')

set -g OSC_END (printf '\a')

set -g OSC_PARAM_SEPARATOR ';'

set -g RESET_GRID_OSC (printf '\e]9279\a')

if test -n "$WARP_INITIAL_WORKING_DIR"
    cd "$WARP_INITIAL_WORKING_DIR" >/dev/null 2>&1
    set -e WARP_INITIAL_WORKING_DIR
end

# Append additional PATH entries if provided via WARP_PATH_APPEND.
if test -n "$WARP_PATH_APPEND"
    set -gx --path PATH "$PATH:$WARP_PATH_APPEND"
    set -e WARP_PATH_APPEND
end

function warp_send_json_message
    # Sends a message to the controlling terminal as a DSC control sequence.
    set -l escaped_json (warp_hex_encode_string "$argv")
    if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]
        echo -n "$OSC_START$DCS_JSON_MARKER$OSC_PARAM_SEPARATOR$escaped_json$OSC_END"
    else
        echo -n "$DCS_START$DCS_JSON_MARKER$escaped_json$DCS_END"
    end
end

function warp_maybe_send_reset_grid_osc
    # Note that $WARP_USING_WINDOWS_CON_PTY is set in the init shell script.
    if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]
        printf $RESET_GRID_OSC
    end
end


# warp_hex_encode_string hex-encodes the given string with `od`.
function warp_hex_encode_string 
  echo "$argv" | od -An -v -tx1 | command tr -d ' \n'
end

# A list of PIDs for running in-band command(s). This is used to kill running
# in-band commands in preexec for a user command, so they do not interfere with
# user command output.
set -g _warp_generator_pids ''

# Runs the given command in the background, records its PID in
# _WARP_GENERATOR_PIDS_STARTED_TMP_FILE, and adds its PID from the file when
# the job is completed.
#
# Usage:
#   _warp_run_generator_command_internal <command_id> '<command>'
#
# The first argument is the command's ID, which is included in the DCS string sent
# to the rust app. The second argument is the command string itself.
function  _warp_run_generator_command_internal
    set -l command_id $argv[1]
    set -l command (string join -- ' ' (string escape $argv[2]))
    # Fish cannot run shell functions in the background, so in order to run
    # the command in the the background we spawn a fish process, inlining the
    # code necessary to write the command's output OSC to the rust app. This
    # differs from the bash and zsh implementations, which rely on running 
    # functions in the background.
    #
    # The `IFS` has to be set to the empty string to prevent fish from converting
    # the command output into a list by splitting output on newlines (fish splits
    # strings on $IFS into lists). Essentially, setting IFS to the empty string
    # allows us to preserve newlines in command output (that might be relied
    # upon) for parsing output.
    #
    # N.B. Fish shell variables cannot contain null characters, so the command output must be
    # immediately hex encoded before being stored in a variable.
    fish -c "
        set -l warp_using_windows_con_pty $WARP_USING_WINDOWS_CON_PTY;
        set -l reset_grid_osc $RESET_GRID_OSC;
        function warp_maybe_send_reset_grid_osc
            if [ \"\$warp_using_windows_con_pty\" = true ]
                printf \$reset_grid_osc
            end
        end
        set -l OSC_START_GENERATOR_OUTPUT \$(printf '\e]9277;A\a')
        set -l OSC_END_GENERATOR_OUTPUT \$(printf '\e]9277;B\a')
        set -l command_id $command_id;
        set -l command $command;
        set -l IFS;
        begin
          echo -n \"\$command_id;\"
          eval \$command 2>&1
          echo -n \";\$status\"
        end | od -An -v -tx1 | command tr -d ' \n' | read -lz hex_encoded_message
        set -l LC_ALL \"C\"
        set -l byte_count (string length \"\$hex_encoded_message\")
        echo -n \"\$OSC_START_GENERATOR_OUTPUT\$byte_count;\$hex_encoded_message\$OSC_END_GENERATOR_OUTPUT\"
        warp_maybe_send_reset_grid_osc" 2> /dev/null &
        
    set -l command_pid $last_pid
    set -a _warp_generator_pids $command_pid

    # Remove the command's PID from _warp_generator_pids when the command exits.
    function on_command_{$command_pid}_finish --on-process-exit $command_pid --inherit-variable command_pid
        set -g _warp_generator_pids (string replace $command_pid '' $_warp_generator_pids)

        # Erase this function after the pids list is updated above so we don't create an infinite number of
        # functions that could pollute the user's context (nested functions are still existing in the global
        # scope).
        functions -e on_command_{$command_pid}_finish

        # Note: If we're on windows, we send a reset grid to erase any cursor mutations caused by
        # the in-band command.
        warp_maybe_send_reset_grid_osc
    end
end

# Executes a generator command in the background, where the first argument is
# the "command ID" assigned by the Rust app and the second-nth arguments
# specify the command to be run.
#
# Note that the command string should be single-quoted so any variables are
# not substituted until the command string is actually evaluated.
#
# Usage:
#   warp_run_generator_command <command_id> '<command> <arg1> ... <argn>'
function warp_run_generator_command
    # Setting this environment variable allows warp_precmd to detect if a generator
    # command or a user command has just completed.
    set -g _WARP_GENERATOR_COMMAND 1
    _warp_run_generator_command_internal $argv
end

# Run before a command is executed.
function warp_preexec --on-event fish_preexec
    set -l command (warp_escape_json "$argv")
    warp_send_json_message "{\"hook\": \"Preexec\", \"value\": {\"command\": \"$command\"}}"
    warp_maybe_send_reset_grid_osc

    # If this preexec is called for user command, kill ongoing generator command jobs.
    if test (! string match -q "warp_run_generator_command*" $argv[1])
        for pid in $_warp_generator_pids
            # Suppress stderr output; kill writes to stderr if any of the given
            # PIDS are not running (which might rarely be the case due to race
            # conditions in checking which PIDS to cancel and this kill command.
            kill -9 $pids >/dev/null 2>/dev/null
        end
        set -g _warp_generator_pids ''
    end
end

# The git prompts git commands are read-only and should not interfere with
# other processes. This environment variable is equivalent to running with `git
# --no-optional-locks`, but falls back gracefully for older versions of git.
# See git(1) for and git-status(1) for a description of that flag.
#
# We wrap in a local function instead of exporting the variable directly in
# order to avoid interfering with manually-run git commands by the user.
function warp_git
    GIT_OPTIONAL_LOCKS=0 command git $argv
end

# Wrap fish prompt function output with OSC prompt marker sequences,
# so that we can direct the prompt bytes to the appropriate grids.
function warp_update_prompt_vars
  # Back up the original fish_prompt if not already done
  if not functions -q warp_original_fish_prompt
    functions -c fish_prompt warp_original_fish_prompt
  end

  # Back up the original fish_right_prompt if it exists and not already backed up
  if functions -q fish_right_prompt; and not functions -q warp_original_fish_right_prompt
    functions -c fish_right_prompt warp_original_fish_right_prompt
  end

  # If not honoring PS1, set both prompts to be empty
  if test "$WARP_HONOR_PS1" = "0"
    function fish_prompt; echo -n ""; end
    function fish_right_prompt; echo -n ""; end
  # If honoring PS1, add prefix/suffix to both prompts
  else

    function end_prompt        
      echo -n (printf '\x1b')
      if test "$WARP_HONOR_PS1" != "1" && [ "$WARP_USING_WINDOWS_CON_PTY" = true ]
        echo -n "]133;B$RESET_GRID_OSC"
      else
        echo -n ']133;B'
      end
      echo -n (printf '\x07')
    end

    # Redefine fish_prompt to include prefix and suffix
    function fish_prompt
      echo -n (printf '\x1b')
      echo -n ']133;A'
      echo -n (printf '\x07')
      warp_original_fish_prompt
      end_prompt
    end

    # Check if warp_original_fish_right_prompt was backed up before redefining fish_right_prompt
    if functions -q warp_original_fish_right_prompt
      function fish_right_prompt
        echo -n (printf '\x1b')
        echo -n ']133;P;k=r'
        echo -n (printf '\x07')
        warp_original_fish_right_prompt
        end_prompt
      end
    end
  end
end

# Changes the WARP_HONOR_PS1 variable to 1, to indicate we want to use the user's custom prompt. Restores
# the original fish prompt functions (which we set to empty for Warp prompt) by calling warp_update_prompt_vars
# to refresh the prompt. We force a repaint of the prompt to ensure the change is reflected immediately.
function warp_change_prompt_modes_to_ps1
  set -x WARP_HONOR_PS1 "1"

  # Restores fish_prompt and fish_right_prompt.
  warp_update_prompt_vars
  # Forces a repaint of the current prompt to ensure the change is reflected immediately.
  commandline -f repaint
end

# Changes the WARP_HONOR_PS1 variable to 0, to indicate we want to use the Warp prompt. Saves and clears
# the fish prompt functions (which we set to empty for Warp prompt) by calling warp_update_prompt_vars
# to refresh the prompt. We force a repaint of the prompt to ensure the change is reflected immediately.
function warp_change_prompt_modes_to_warp_prompt
  set -x WARP_HONOR_PS1 "0"

  # Updates fish_prompt and fish_right_prompt to be empty.
  warp_update_prompt_vars
  # Forces a repaint of the current prompt to ensure the change is reflected immediately.
  commandline -f repaint
end

set block_id 0
# Run before the prompt is displayed. We also need to trigger this on "fish_posterror", as
# submitting a command containing a syntax error will not trigger "fish_preexec" or "fish_prompt".
function warp_precmd --on-event fish_prompt --on-event fish_posterror
    # Handle prompt behavior (we do this first to make sure the exit status is from the command,
    # rather than from our own code)
    set -l exit_code $status

    # This function is triggered by both "fish_prompt" and "fish_posterror" events. If it was
    # "fish_prompt", then $status will be properly set. If it was "fish_posterror", it won't be.
    # We want to set the exit code to 1 for errors, but there is no direct way to check which event
    # triggered this function call. The length of $argv actually tells us, as "fish_posterror"
    # passes the erroneous command while "fish_prompt" passes nothing.
    if test (count $argv) -gt 0
        set exit_code 1
    end

    warp_send_json_message "{\"hook\": \"CommandFinished\", \"value\": {\"exit_code\": $exit_code, \"next_block_id\": \"precmd-$WARP_SESSION_ID-$block_id\"}}"
    warp_maybe_send_reset_grid_osc

    set block_id (math $block_id + 1)

    if ! test -z $_WARP_GENERATOR_COMMAND
        set -e _WARP_GENERATOR_COMMAND
        set -l escaped_json "{\"hook\": \"Precmd\", \"value\": {
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
        warp_send_json_message $escaped_json
        return 0
    end

    # Fish's vi mode causes a conflict with the input reporting binding.
    # Set the bindings back for this session. Note that just calling fish_default_key_bindings
    # affects other sessions so we do it this way instead.
    set -g fish_key_bindings fish_default_key_bindings

    # Do not use kill-line because of the kill ring interation.
    bind \cP "commandline ''"

    # We use the ESC-p bindkey for this ("p" for PS1/custom prompt).
    bind \ep warp_change_prompt_modes_to_ps1

    # We use the ESC-w bindkey for this ("w" for Warp prompt).
    bind \ew warp_change_prompt_modes_to_warp_prompt

    bind \ei warp_report_input

    # Define local variables in appropriate outer block for fish variable scoping.
    # See https://stackoverflow.com/a/53685510.
    set -l escaped_prompt
    set -l escaped_right_prompt

    set -l escaped_pwd
    if set -q WSL_DISTRO_NAME
        # In WSL, avoid symlinks b/c on Windows `std::fs` is unable to resolve symlink inside WSL containers.
        set escaped_pwd (warp_escape_json (pwd -P))
    else
        set escaped_pwd (warp_escape_json $PWD)
    end

    set -l escaped_virtual_env ""
    set -l escaped_conda_env ""
    set -l escaped_node_version ""
    set -l escaped_git_head ""
    set -l escaped_git_branch ""

    # Only fill these fields once we've finished bootstrapping, as the
    # blocks created during the bootstrap process don't have visible
    # prompts, and we don't want to invoke `git` before we've sourced the
    # user's rcfiles and have a fully-populated PATH.
    if test -n "$WARP_BOOTSTRAPPED"
      if test -n "$VIRTUAL_ENV"
          set escaped_virtual_env (warp_escape_json "$VIRTUAL_ENV")
      end
      if test -n "$CONDA_DEFAULT_ENV"
          set escaped_conda_env (warp_escape_json "$CONDA_DEFAULT_ENV")
      end
      
        # Get Node.js version if node is available and we're in a Node.js project
        if command -v node > /dev/null 2>&1
            # Check for package.json in current directory and parent directories
            set current_dir (pwd)
            set found_package_json false
            set package_json_dir ""
            while test "$current_dir" != "/"
                if test -f "$current_dir/package.json"
                    set found_package_json true
                    set package_json_dir "$current_dir"
                    break
                end
                set current_dir (dirname "$current_dir")
            end
            
            # Only show node version if package.json is within a git repository
            if test "$found_package_json" = true
                set git_dir "$package_json_dir"
                set in_git_repo false
                while test "$git_dir" != "/"
                    if test -d "$git_dir/.git"
                        set in_git_repo true
                        break
                    end
                    set git_dir (dirname "$git_dir")
                end
                
                if test "$in_git_repo" = true
                    set node_version (node --version 2>/dev/null)
                    if test -n "$node_version"
                        set escaped_node_version (warp_escape_json "$node_version")
                    end
                end
            end
        end

      set -l git_branch ""
      set -l git_head ""
      if command -q git
          set git_branch (warp_git symbolic-ref --short HEAD 2> /dev/null)
          if test -z "$git_branch"
              # Fallback to the git commit hash if we aren't on a named branch.
              set git_head (warp_git rev-parse --short HEAD 2> /dev/null)
          else
              set git_head "$git_branch"
          end
      end
      set escaped_git_head (warp_escape_json "$git_head")
      set escaped_git_branch (warp_escape_json "$git_branch")
    end

    warp_update_prompt_vars
    # This is used solely for prompt previews, when we're using prompt markers with combined grid.
    # We need to use this since fish does not have a way to ignore printable characters for cursor
    # positioning (unlike zsh/bash), so we need a separate mechanism to send the prompt to Warp
    # in the case of Warp prompt (for previewing the PS1). We send an escaped version of the raw prompt
    # bytes via a hex string (in a JSON payload) to Warp.
    # Note that we are CALLING the `warp_original_fish_prompt` function on the next line and assigning the
    # outputted string to the local variable `raw_prompt_for_preview`.
    set -l raw_prompt_for_preview (warp_original_fish_prompt)
    # We encode the prompt as a hex string to pass it to Warp.
    set escaped_prompt (warp_escape_prompt "$raw_prompt_for_preview")

    set -l escaped_json
    if test "$WARP_HONOR_PS1" = "1"
      # Don't send lprompt or rprompt in this case - we'll use prompt markers for both directly!
      set escaped_json "{\"hook\": \"Precmd\", \"value\": {
      \"pwd\": \"$escaped_pwd\",
      \"ps1\": \"\",
      \"rprompt\": \"\",
      \"git_head\": \"$escaped_git_head\",
      \"git_branch\": \"$escaped_git_branch\",
      \"virtual_env\": \"$escaped_virtual_env\",
      \"conda_env\": \"$escaped_conda_env\",
      \"node_version\": \"$escaped_node_version\",
      \"session_id\": $WARP_SESSION_ID
      }}"
    else
      # We send an lprompt to use for prompt preview purposes only (we still use prompt markers for active prompts).
      set escaped_json "{\"hook\": \"Precmd\", \"value\": {
      \"pwd\": \"$escaped_pwd\",
      \"ps1\": \"$escaped_prompt\",
      \"rprompt\": \"\",
      \"git_head\": \"$escaped_git_head\",
      \"git_branch\": \"$escaped_git_branch\",
      \"virtual_env\": \"$escaped_virtual_env\",
      \"conda_env\": \"$escaped_conda_env\",
      \"node_version\": \"$escaped_node_version\",
      \"session_id\": $WARP_SESSION_ID
      }}"
    end
    warp_send_json_message $escaped_json
end

function warp_escape_prompt
    # To match the implementation of PS1 support in bash / zsh, we use the same method of encoding
    # the prompt as a hex string so that it can be interpreted on the Warp side
    # Note: before converting the prompt to a hex string, we remove any multi-line newlines and
    # replace them with a single space (to avoid prompts that span multiple empty lines)
    echo "$argv" | command tr '\n\n' ' ' | command od -An -v -tx1 | command tr -d ' \n'
end

# Escapes string(s) passed as input to be JSON-serializable.
#
# Specifically, special characters like backspace, tab, form feed, carriage return, and newlines
# are replaced with escaped equivalents. Double quotes and literal backslash characters are also
# backslash-escaped.
function warp_escape_json
    # Explanation of the sed replacements (each command is separated by a `;`):
    # s/(["\\])/\\\1/g - Replace all double-quote (") and backslash (\) characters with the escaped versions (\" and \\)
    # s/\b/\\b/g - Replace all backspace characters with \b
    # s/\t/\\t/g - Replace all tab characters with \t
    # s/\f/\\f/g - Replace all form feed characters with \f
    # s/\r/\\r/g - Replace all carriage return characters with \r
    # $!s/$/\\\\n/ - Replace any newlines (except the last trailing newline) with the literal \n character
    #
    # Additional note: In Fish, backslashes still need to be escaped in single quoted strings. So each `\\` below
    # represents a single `\` in the above regex explanation. We also close the single-quoted string in order to
    # insert literal characters (Fish supports literals with escapes when not quoting)
    #
    # We also use 'command sed' to ensure that we aren't accidentally executing a function named 'sed'
    string join \n $argv | command sed -E 's/(["\\\\])/\\\\\\1/g; s/'\b'/\\\\b/g; s/'\t'/\\\\t/g; s/'\f'/\\\\f/g; s/'\r'/\\\\r/g; $!s/$/\\\\n/' | command tr -d '\n'
end

function warp_bootstrapped
  set -l histfile_directory
  set histfile_directory "$XDG_DATA_HOME"
  if test -z "$histfile_directory"
        set histfile_directory "$HOME/.local/share"
  end
  set -l escaped_histfile (warp_escape_json "$histfile_directory/fish/fish_history")

  set -l vi_mode_enabled ""
  if [ "$fish_key_bindings" = "fish_vi_key_bindings" ]
      set vi_mode_enabled "1"
  end

  set -l kernel_name (uname)
  if test -n "$kernel_name"
    if [ "$kernel_name" = "Darwin" ]
      set os_category "MacOS"
    else if [ "$kernel_name" = "Linux" ]
      set os_category "Linux"
      set -l default_os_release_filepath "/etc/os-release"
      set -l fallback_os_release_filepath "/usr/lib/os-release"
      # We first try /etc/os-release and then try /usr/lib/os-release as a fallback.
      if test -f "$default_os_release_filepath"
        set os_release_file "$default_os_release_filepath";
      else if test -f "$fallback_os_release_filepath"
        set os_release_file "$fallback_os_release_filepath";
      end
      if test -f "$os_release_file"
        set linux_distribution (cat $os_release_file | sed -nE 's/^NAME="(.*)"$/\1/p')
      end
    end
  end

  set -l escaped_abbr (warp_escape_json (abbr --show))
  set -l escaped_aliases (warp_escape_json (alias))
  set -l env_var_names (warp_escape_json (set --names))
  set -l function_names (warp_escape_json (functions -an))
  set -l escaped_builtins (warp_escape_json (builtin -n))
  # Note "keywords" is set to an empty string since fish includes keywords as a
  # part of its builtins (e.g. "for", "while", etc.).
  set -l escaped_editor (warp_escape_json "$EDITOR")
  set -l escaped_shell_path (warp_escape_json (status fish-path))
  set -l escaped_json "{\"hook\": \"Bootstrapped\", \"value\": {\"histfile\": \"$escaped_histfile\", \"shell\": \"fish\", \"home_dir\": \"$HOME\", \"path\": \"$PATH\", \"editor\": \"$escaped_editor\", \"abbreviations\": \"$escaped_abbr\", \"aliases\": \"$escaped_aliases\", \"function_names\": \"$function_names\", \"env_var_names\": \"$env_var_names\", \"builtins\": \"$escaped_builtins\", \"keywords\": \"\", \"shell_version\": \"$FISH_VERSION\", \"vi_mode_enabled\": \"$vi_mode_enabled\", \"os_category\": \"$os_category\", \"linux_distribution\": \"$linux_distribution\", \"wsl_name\": \"$WSL_DISTRO_NAME\", \"shell_path\": \"$escaped_shell_path\"}}"
  warp_send_json_message $escaped_json
end

function warp_init_shell
    set -l  init_shell "{\"hook\": \"InitShell\", \"value\": {\"shell\": \"$argv\"}}"
    warp_hex_encode_string "$init_shell"
end

# Add a key binding to report the current input buffer to Warp. We can override
# any user-defined binds here because user input goes through Warp's editor, not
# the fish line editor.
# This is arbitrarily bound to ESC-i in all supported shells ("i" for input).
# Binding to ESC-1 caused bootstrap failures with vi keybindings.
function warp_report_input
    set -l escaped_input (warp_escape_json (commandline))
    warp_send_json_message "{ \"hook\": \"InputBuffer\", \"value\": { \"buffer\": \"$escaped_input\" } }"
    # This prevents fish from rendering typeahead as background output once we've collected it.
    commandline ''
end

function clear
    warp_send_json_message "{\"hook\": \"Clear\", \"value\": {}}"
end

function warp_finish_update
  set -l update_id "$argv[1]"
  warp_send_json_message "{\"hook\": \"FinishUpdate\", \"value\": { \"update_id\": \"$update_id\"}}"
end


# Check if the warp apt source file has been renamed to `warpdotdev.list.distUpgrade` due to an ubuntu version update.
# If this occurred, we want to rename the source file back to `warpdotdev.list` to ensure updates can proceed.
# We purposefully skip this if either the `warpdotdev.list` file already exists (indicating that the user has already
# done this themselves) _or_ if a `warpdotdev.sources` file exists (which is the new Deb822 format for source files).
# The `.sources` file could only exist if a user manually created it; Ubuntu doesn't create one automatically for the
# warp source file due to a bug in its update flow where it considers our source file to be "invalid" because it
# contains a `signed-by` key.
function warp_handle_dist_upgrade
  set -l source_file_name "$argv[1]"

  # The `apt-config shell` command outputs an environment variable assignment in POSIX-compliant syntax. Therefore,
  # we need to run this from within an sh shell to actually get the correct directory for the sources dir.
  set -l APT_SOURCESDIR (command sh -c 'eval $(apt-config shell APT_SOURCESDIR "Dir::Etc::sourceparts/d"); echo $APT_SOURCESDIR')

if not test -e $APT_SOURCESDIR$source_file_name.list; and not test -e $APT_SOURCESDIR$source_file_name.sources; and test -e $APT_SOURCESDIR$source_file_name.list.distUpgrade
      # DO NOT DO THIS. We should never run a command for user with `sudo`. The only reason this is safe here is because
      # we insert this function into the input for the user to determine if they want to execute (we never run it on
      # their behalf without their permission).  To be transparent about what is being executed with sudo, we echo out the
      # command we're about to run.
      echo "Executing: sudo cp \"$APT_SOURCESDIR$source_file_name.list.distUpgrade\" \"$APT_SOURCESDIR$source_file_name.list\""
      sudo cp "$APT_SOURCESDIR$source_file_name.list.distUpgrade" "$APT_SOURCESDIR$source_file_name.list"
  end
end

# The SSH logic only applies to local sessions, because we don't yet have support for bootstrapping
# recursive SSH sessions.
if test "$WARP_IS_LOCAL_SHELL_SESSION" = "1"
    function is_interactive_ssh_session
        # Parse through all ssh options, as defined in the ssh man pages.  Send
        # stderr to /dev/null to silence argparse output when an option is invalid.
        argparse '1' '2' '4' '6' 'A' 'a' 'C' 'f' 'g' 'K' 'k' 'M' 'N' 'n' 'q' 's' 'T' 't' 'V' 'v' 'X' 'x' 'Y' 'y' 'b=' 'c=' 'D=' 'e=' 'F=' 'i=' 'L=' 'l=' 'm=' 'O=' 'o=' 'p=' 'R=' 'S=' 'W=' 'w=' -- $argv 2>/dev/null
        # If argparse returned a non-success value, propagate it up.
        or return

        if [ $_flag_T ]
            # -T disables pty allocation (aka a non-interactive session)
            return 1
        end
        if [ $_flag_W ]
            # -W implies -T
            return 1
        end

        # If there is more than one positional argument, the user is attempting to
        # run a command, not start an interactive session.  If there is less than
        # one positional argument, the user should be shown the usage text.
        if [ (count $argv) -ne 1 ]
            return 1
        end
    end

    function warp_ssh_helper
        set -l init_shell_zsh (warp_init_shell "zsh")
        set -l init_shell_bash (warp_init_shell "bash")
        # Hex-encode the ZSH environment script we use to bootstrap remote zsh b/c it contains control characters
        # We decode on the SSH server using xxd if its available, otherwise fall back to a for-loop over each byte
        # and use printf to convert back to plaintext
        set -l zsh_env_script (printf '%s' 'unsetopt ZLE; unset RCS; unset GLOBAL_RCS; WARP_SESSION_ID="$(command -p date +%s)$RANDOM"; WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@; WARP_HONOR_PS1='$WARP_HONOR_PS1'; _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || uname -n); _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER); _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"zsh\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n"); printf '"'"'\x1b\x50\x24\x64%s\x9c'"'"' $_msg; unset _hostname _user _msg' | command od -An -v -tx1 | command tr -d ' \n')

        # Note that in this command, we're passing a string to the remote shell. Any variable expansions need to be
        # escaped with "''" to avoid the local shell from expanding them before they're passed to the remote shell.
        # We check the SHELL env var and use shell string manipulation to get the contents after the last slash to
        # determine what shell is the login shell on the remote machine.  We perform a preliminary check to see if
        # the remote shell is the Bourne shell to avoid asking it to parse later lines that use syntax it doesn't
        # support.
        set -l remote_command "
export TERM_PROGRAM='WarpTerminal'
test -n '$WARP_CLIENT_VERSION' && export WARP_CLIENT_VERSION='$WARP_CLIENT_VERSION'
# Only forward the protocol version if it was set locally (i.e. the HOANotifications feature flag is on).
test -n '$WARP_CLI_AGENT_PROTOCOL_VERSION' && export WARP_CLI_AGENT_PROTOCOL_VERSION='$WARP_CLI_AGENT_PROTOCOL_VERSION'
hook="'$(printf "{\"hook\": \"SSH\", \"value\": {\"socket_path\": \"'$SSH_SOCKET_DIR/$WARP_SESSION_ID'\", \"remote_shell\": \"%s\"}}" "${SHELL##*/}" | command od -An -v -tx1 | command tr -d " \n")'"
printf '$DCS_START$DCS_JSON_MARKER%s$DCS_END' "'$hook'"

if test "'"${SHELL##*/}" != "bash" -a "${SHELL##*/}" != "zsh"'"; then
  # Emulate the SSHD logic to print the MotD. Because the Warp SSH wrapper passes
  # a command to run, SSHD does a quiet login, updating utmp and other login
  # state, but not printing the MotD. For bash and zsh, this is instead handled
  # by our bootstrap script.
  if test ! -e "'$HOME/.hushlogin'"; then
    # This uses an if-else chain instead of a for-loop to avoid expansion issues on older shells.
    if test -r /etc/motd; then
      cat /etc/motd
    elif test -r /run/motd; then
      cat /run/motd
    elif test -r /run/motd.dynamic; then
      cat /run/motd.dynamic
    elif test -r /usr/lib/motd; then
      cat /usr/lib/motd
    elif test -r /usr/lib/motd.dynamic; then
      cat /usr/lib/motd.dynamic
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
      stty raw
      HISTCONTROL=ignorespace
      HISTIGNORE=" *"
      WARP_SESSION_ID="$(command -p date +%s)$RANDOM"
      WARP_HONOR_PS1="'$WARP_HONOR_PS1'"
      _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || uname -n)
      _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER)
      _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"bash\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n")'"
      WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@
      if [[ "'$OS'" == Windows_NT ]]; then WARP_IN_MSYS2=true; else WARP_IN_MSYS2=false; fi
      printf '\''"'\eP$d%s\x9c'"'\'' \""'$_msg'"\"'
      unset _hostname _user _msg
    )
    ;;
zsh) WARP_TMP_DIR="'$(mktemp -d warptmp.XXXXXX)'"
local ZSH_ENV_SCRIPT='$zsh_env_script'
if [[ "'$?'" == 0 ]]; then
  if command -v xxd >/dev/null 2>&1; then
    echo "'$ZSH_ENV_SCRIPT'" | command xxd -p -r > "'$WARP_TMP_DIR'"/.zshenv
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
        set -l remote_command_octal (printf '%s' "$remote_command" | command od -An -v -to1 | command tr -d ' \n' | command sed 's/.../\\&/g')

        # Run the POSIX bootstrap payload through sh so remote Fish login shells do not parse it.
        command ssh -o ControlMaster=yes -o ControlPath=$SSH_SOCKET_DIR/$WARP_SESSION_ID \
        -t $argv \
        "command sh -c 'printf \"%b\" \"\$1\" | command sh' sh '$remote_command_octal'"
    end

    function ssh
        if is_interactive_ssh_session $argv
            warp_send_json_message '{"hook": "PreInteractiveSSHSession", "value": {}}'

            if [ "$WARP_USE_SSH_WRAPPER" = "1" ]
                if test $WARP_SHELL_DEBUG_MODE
                    set -g TRACE_FLAG_IF_WARP_SHELL_DEBUG_MODE "-x"
                else
                    set -g TRACE_FLAG_IF_WARP_SHELL_DEBUG_MODE ""
                end
                warp_ssh_helper $argv
            else
                command ssh $argv
            end
        else
            command ssh $argv
        end
    end
end

warp_precmd

# Print the MotD if this is a login shell. Normally, login(1) or pam_motd(8)
# would do this. However, Warp does not use login(1) for local sessions and for
# remote sessions, SSHD thinks it is starting a non-interactive session, so it
# does not print PAM messages.
if status --is-login
  and test ! -e "$HOME/.hushlogin"
  for motd_file in /etc/motd /run/motd /run/motd.dynamic /usr/lib/motd /usr/lib/motd.dynamic;
    if test -r "$motd_file"
      command cat "$motd_file"
      break
    end
  end
end

warp_bootstrapped

set -g WARP_BOOTSTRAPPED 1
set -g fish_private_mode $saved_fish_private_mode
end
