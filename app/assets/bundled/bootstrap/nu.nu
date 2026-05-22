# Note that WARP_SESSION_ID is expected to have been set when executing commands
# to emit the InitShell payload, which includes the session ID.
if (($env.WARP_BOOTSTRAPPED? | default "") == "") {
    if (($env.WARP_INITIAL_WORKING_DIR? | default "") != "") {
        try { cd $env.WARP_INITIAL_WORKING_DIR } catch { }
        hide-env WARP_INITIAL_WORKING_DIR
    }

    if (($env.WARP_PATH_APPEND? | default "") != "") {
        let path_append = ($env.WARP_PATH_APPEND | split row (char esep))
        if (($env.PATH | describe) | str contains "list") {
            $env.PATH = ($env.PATH | append $path_append)
        } else {
            $env.PATH = $"($env.PATH)(char esep)($env.WARP_PATH_APPEND)"
        }
        hide-env WARP_PATH_APPEND
    }

    def warp_session_id [] {
        $env.WARP_SESSION_ID | into int
    }

    def warp_hex_encode_string [message] {
        $message | ^od -An -v -tx1 | ^tr -d ' \n'
    }

    def warp_maybe_send_reset_grid_osc [] {
        if (($env.WARP_USING_WINDOWS_CON_PTY? | default "false") == "true") {
            print -n $"(ansi escape)]9279(char bel)"
        }
    }

    def warp_send_json_message [message] {
        let encoded_message = (warp_hex_encode_string $message)
        if (($env.WARP_USING_WINDOWS_CON_PTY? | default "false") == "true") {
            print -n $"(ansi escape)]9278;d;($encoded_message)(char bel)"
        } else {
            print -n $"(ansi escape)P$d($encoded_message)(ansi escape)\\"
        }
    }

    def warp_send_hook [hook value] {
        let message = ({ hook: $hook, value: $value } | to json -r)
        warp_send_json_message $message
    }

    def warp_send_generator_output_osc_pre_hex_encoded [hex_encoded_message] {
        let byte_count = ($hex_encoded_message | str length)
        print -n $"(ansi escape)]9277;A(char bel)($byte_count);($hex_encoded_message)(ansi escape)]9277;B(char bel)"
        warp_maybe_send_reset_grid_osc
    }

    def --env warp_run_generator_command [command_id command] {
        $env._WARP_GENERATOR_COMMAND = "1"
        let result = (^nu --commands $command | complete)
        let output = $"($command_id;)($result.stdout)($result.stderr);($result.exit_code)"
        let hex_encoded_message = (warp_hex_encode_string $output)
        warp_send_generator_output_osc_pre_hex_encoded $hex_encoded_message
    }

    def warp_preexec [command] {
        warp_send_hook "Preexec" { command: $command }
        warp_maybe_send_reset_grid_osc
    }

    def warp_path_string [] {
        let path = ($env.PATH? | default [])
        if (($path | describe) | str contains "list") {
            $path | str join (char esep)
        } else {
            $path | into string
        }
    }

    def warp_command_names_by_type [command_type] {
        try {
            scope commands | where type == $command_type | get name | uniq | str join (char nl)
        } catch {
            ""
        }
    }

    def warp_aliases [] {
        try {
            scope aliases | each {|alias| $"($alias.name)\t($alias.expansion)" } | str join (char nl)
        } catch {
            ""
        }
    }

    def warp_os_category [] {
        let os_name = ($nu.os-info.name? | default "")
        if (($os_name | str downcase) | str contains "macos") {
            "MacOS"
        } else if (($os_name | str downcase) | str contains "linux") {
            "Linux"
        } else if (($os_name | str downcase) | str contains "windows") {
            "Windows"
        } else {
            ""
        }
    }

    def warp_linux_distribution [] {
        try {
            let os_release_path = if ("/etc/os-release" | path exists) { "/etc/os-release" } else { "/usr/lib/os-release" }
            if ($os_release_path | path exists) {
                open $os_release_path | lines | where {|line| $line | str starts-with "NAME=" } | first | str replace --regex '^NAME="?([^"]*)"?$' '$1'
            } else {
                ""
            }
        } catch {
            ""
        }
    }

    def --env warp_precmd [] {
        let exit_code = ($env.LAST_EXIT_CODE? | default 0)
        let block_id = ($env.WARP_BLOCK_ID? | default "0" | into int)
        $env.WARP_BLOCK_ID = (($block_id + 1) | into string)
        warp_send_hook "CommandFinished" {
            exit_code: $exit_code,
            next_block_id: $"precmd-($env.WARP_SESSION_ID)-($block_id)"
        }
        warp_maybe_send_reset_grid_osc

        if (($env._WARP_GENERATOR_COMMAND? | default "") != "") {
            hide-env _WARP_GENERATOR_COMMAND
            warp_send_hook "Precmd" {
                pwd: "",
                ps1: "",
                git_head: "",
                git_branch: "",
                virtual_env: "",
                conda_env: "",
                node_version: "",
                session_id: (warp_session_id),
                is_after_in_band_command: true
            }
            return
        }

        let git_branch = (try { ^git symbolic-ref --short HEAD | str trim } catch { "" })
        let git_head = if $git_branch != "" { $git_branch } else { try { ^git rev-parse --short HEAD | str trim } catch { "" } }

        warp_send_hook "Precmd" {
            pwd: (pwd),
            ps1: "",
            rprompt: "",
            git_head: $git_head,
            git_branch: $git_branch,
            virtual_env: ($env.VIRTUAL_ENV? | default ""),
            conda_env: ($env.CONDA_DEFAULT_ENV? | default ""),
            node_version: "",
            session_id: (warp_session_id)
        }
    }

    def warp_bootstrapped [] {
        let aliases = (warp_aliases)
        let functions = (warp_command_names_by_type custom)
        let builtins = (warp_command_names_by_type "built-in")
        let keywords = (warp_command_names_by_type keyword)
        let env_var_names = ($env | columns | str join (char nl))
        let version_info = (try { version } catch { {} })
        let shell_version = (try { $version_info.version } catch { "" })
        let shell_path = (try { $nu.current-exe | into string } catch { "" })
        let histfile = (try { $nu.history-path | into string } catch { "" })

        warp_send_hook "Bootstrapped" {
            histfile: $histfile,
            session_id: (warp_session_id),
            shell: "nu",
            home_dir: ($nu.home-dir? | default ($env.HOME? | default "")),
            path: (warp_path_string),
            cdpath: "",
            editor: ($env.EDITOR? | default ""),
            env_var_names: $env_var_names,
            abbreviations: "",
            aliases: $aliases,
            function_names: $functions,
            builtins: $builtins,
            keywords: $keywords,
            shell_version: $shell_version,
            shell_options: "",
            vi_mode_enabled: "",
            os_category: (warp_os_category),
            linux_distribution: (warp_linux_distribution),
            wsl_name: ($env.WSL_DISTRO_NAME? | default ""),
            shell_path: $shell_path
        }
    }

    def clear [] {
        warp_send_hook "Clear" {}
    }

    def warp_finish_update [update_id] {
        warp_send_hook "FinishUpdate" { update_id: $update_id }
    }

    def warp_handle_dist_upgrade [source_file_name] {
        let apt_sources_dir = (try { ^sh -c 'eval $(apt-config shell APT_SOURCESDIR "Dir::Etc::sourceparts/d"); echo $APT_SOURCESDIR' | str trim } catch { "" })
        if $apt_sources_dir != "" {
            let list_path = $"($apt_sources_dir)($source_file_name).list"
            let sources_path = $"($apt_sources_dir)($source_file_name).sources"
            let dist_upgrade_path = $"($list_path).distUpgrade"
            if (not ($list_path | path exists)) and (not ($sources_path | path exists)) and ($dist_upgrade_path | path exists) {
                print $"Executing: sudo cp \"($dist_upgrade_path)\" \"($list_path)\""
                sudo cp $dist_upgrade_path $list_path
            }
        }
    }

    if (($env.WARP_HONOR_PS1? | default "0") == "0") {
        $env.PROMPT_COMMAND = ""
        $env.PROMPT_COMMAND_RIGHT = ""
    }

    $env.config = ($env.config | upsert hooks.pre_execution { default [] | append {|| warp_preexec (commandline) } })
    $env.config = ($env.config | upsert hooks.pre_prompt { default [] | append {|| warp_precmd } })

    warp_precmd
    warp_bootstrapped
    $env.WARP_BOOTSTRAPPED = "1"
}
