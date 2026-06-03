#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::path::Path;
use std::{env, fmt};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use url::Url;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;

use crate::agent::OutputFormat;

#[cfg(windows)]
mod process_handle;

pub mod artifact;
pub mod scope;
pub mod skill;
mod sort_order;
pub use sort_order::SortOrderArg;

pub mod agent;
pub mod api_key;
pub mod completions;
pub mod config_file;
mod date_time;
pub mod environment;
pub mod federate;
pub mod harness_support;
pub mod integration;
pub mod json_filter;
pub mod mcp;
pub mod model;
pub mod provider;
pub mod schedule;
pub mod secret;
pub mod share;
pub mod task;
pub const OZ_RUN_ID_ENV: &str = "OZ_RUN_ID";
pub const OZ_PARENT_RUN_ID_ENV: &str = "OZ_PARENT_RUN_ID";
pub const OZ_CLI_ENV: &str = "OZ_CLI";
pub const OZ_HARNESS_ENV: &str = "OZ_HARNESS";
pub const SERVER_ROOT_URL_OVERRIDE_ENV: &str = "WARP_SERVER_ROOT_URL";
pub const WS_SERVER_URL_OVERRIDE_ENV: &str = "WARP_WS_SERVER_URL";
pub const SESSION_SHARING_SERVER_URL_OVERRIDE_ENV: &str = "WARP_SESSION_SHARING_SERVER_URL";

/// Options related to the parent process that spawned this Warp instance.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct ParentOpts {
    /// The ID of the Warp process that spawned this one.
    ///
    /// Used by codepaths that attempt to detect when the parent Warp process
    /// has terminated. Guaranteed to be [`None`] when this is the initial
    /// Warp process, but may also be [`None`] for Warp child processes if the
    /// child process doesn't need to keep track of its parent.
    #[arg(long = "parent-pid", hide = true)]
    pub pid: Option<u32>,

    /// A handle to our parent process.
    ///
    /// Used on Windows for crash recovery instead of parent_pid, as process
    /// IDs can be reused, so a process handle is more robust.
    #[cfg(windows)]
    #[arg(long = "parent-handle", hide = true)]
    pub handle: Option<process_handle::ProcessHandle>,
}

/// Hidden worker args used to scope remote-server proxy/daemon sockets by
/// Warp identity without exposing credentials.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct RemoteServerIdentityArgs {
    /// Non-secret identity partition key for the remote-server daemon.
    #[arg(long = "identity-key", hide = true)]
    pub identity_key: String,
}

/// Global options that apply to all CLI commands.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct GlobalOptions {
    /// API key for server authentication.
    #[arg(long = "api-key", global = true, env = "WARP_API_KEY")]
    pub api_key: Option<String>,

    /// Set the output format.
    #[arg(
        long = "output-format",
        global = true,
        value_enum,
        default_value_t = OutputFormat::Pretty,
        env = "WARP_OUTPUT_FORMAT"
    )]
    pub output_format: OutputFormat,
}

/// Command-line argument parser for the main Warp binary. This is used across all channels.
#[derive(Debug, Default, Parser, Clone)]
#[command(
    name = "oz",
    display_name = "Oz",
    about = r#"The orchestration platform for cloud agents

The Oz CLI is a tool for running, managing, and orchestrating coding agents at scale.
Use the CLI to:
* Launch and inspect cloud agents
* Schedule cloud agents to run in the future
* Manage the environments that cloud agents run in
* Upload secrets to Oz's secure storage"#
)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[clap(flatten)]
    global_options: GlobalOptions,

    /// Enable debug mode.
    #[arg(long = "debug", global = true, help = "Enable debug logging")]
    debug: bool,

    /// Override the server root URL.
    #[arg(
        long = "server-root-url",
        global = true,
        hide = true,
        env = "WARP_SERVER_ROOT_URL"
    )]
    server_root_url: Option<String>,

    /// Override the websocket server URL.
    #[arg(
        long = "ws-server-url",
        global = true,
        hide = true,
        env = "WARP_WS_SERVER_URL"
    )]
    ws_server_url: Option<String>,

    /// Override the session sharing server URL.
    #[arg(
        long = "session-sharing-server-url",
        global = true,
        hide = true,
        env = "WARP_SESSION_SHARING_SERVER_URL"
    )]
    session_sharing_server_url: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    #[clap(flatten)]
    args: AppArgs,
}

/// Flags for the Warp application. Additional binaries, like test runners, may use this type
/// along with their own flags, or convert their flags into an `AppArgs` value.
#[derive(Debug, Default, clap::Args, Clone)]
pub struct AppArgs {
    /// True if this instance of Warp was launched at the end of the auto-update process.
    #[arg(long = "finish-update", hide = true)]
    pub finish_update: bool,

    /// Crash recovery mechanism to use if we detect the parent process terminated.
    #[cfg(enable_crash_recovery)]
    #[arg(long = "crash-recovery-mechanism", value_enum, requires = "ParentOpts")]
    pub crash_recovery_mechanism: Option<RecoveryMechanism>,

    /// Options related to the parent process that spawned this Warp instance.
    #[clap(flatten)]
    pub parent: ParentOpts,

    /// URLs to open in Warp.
    #[arg(hide = true)]
    pub urls: Vec<Url>,
}

impl Args {
    /// Parses command-line arguments from the operating environment. May exit early if arguments
    /// are incorrectly specified.
    pub fn from_env() -> Self {
        cfg_if::cfg_if! {
            // wasm doesn't have any concept of an environment, so skip parsing and return defaults
            if #[cfg(target_family = "wasm")] {
                Args::default()
            } else {
                use clap::FromArgMatches as _;

                // Check for disabled commands before parsing to prevent help from showing (e.g.
                // `warp environment` should not return help text)
                if !FeatureFlag::CloudEnvironments.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "environment" {
                        exit_unrecognized_subcommand("environment");
                    }
                }

                if !FeatureFlag::ProviderCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "provider" {
                        exit_unrecognized_subcommand("provider");
                    }
                }

                if !FeatureFlag::IntegrationCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "integration" {
                        exit_unrecognized_subcommand("integration");
                    }
                }

                if !FeatureFlag::ScheduledAmbientAgents.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "schedule" {
                        exit_unrecognized_subcommand("schedule");
                    }
                }

                if !FeatureFlag::WarpManagedSecrets.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "secret" {
                        exit_unrecognized_subcommand("secret");
                    }
                }

                if !FeatureFlag::OzIdentityFederation.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "federate" {
                        exit_unrecognized_subcommand("federate");
                    }
                }

                if !FeatureFlag::ArtifactCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "artifact" {
                        exit_unrecognized_subcommand("artifact");
                    }
                }

                if !FeatureFlag::APIKeyManagement.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "api-key" {
                        exit_unrecognized_subcommand("api-key");
                    }
                }

                let command = Self::clap_command();

                command.try_get_matches()
                    .and_then(|matches| Self::from_arg_matches(&matches))
                    .unwrap_or_else(|err| {
                        // We attach a console to ensure help and error messages are printed
                        // when using the CLI.
                        #[cfg(windows)]
                        warp_util::windows::attach_to_parent_console();
                        err.exit()
                    })
            }
        }
    }

    /// Construct the [`clap::Command`] that backs `Args`.
    ///
    /// IMPORTANT: use this instead of [`CommandFactory::command`], since we customize the command at runtime.
    pub fn clap_command() -> clap::Command {
        let mut command = <Args as CommandFactory>::command();
        command = localize_clap_command(command);

        // Hide the environment subcommands and --environment flags from help text
        if !FeatureFlag::CloudEnvironments.is_enabled() {
            command = command.mut_subcommand("environment", |c| c.hide(true));
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd
                    .mut_subcommand("run", |run_cmd| {
                        run_cmd.mut_arg("environment", |arg| arg.hide(true))
                    })
                    .mut_subcommand("run-cloud", |cloud_cmd| {
                        cloud_cmd.mut_arg("environment", |arg| arg.hide(true))
                    })
            });
        }

        // Hide the --conversation flag from help text
        if !FeatureFlag::CloudConversations.is_enabled() {
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd
                    .mut_subcommand("run", |run_cmd| {
                        run_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
                    .mut_subcommand("run-cloud", |cloud_cmd| {
                        cloud_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
            });
        }

        if !FeatureFlag::AmbientAgentsCommandLine.is_enabled() {
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd.mut_subcommand("run-cloud", |c| c.hide(true))
            });
        }

        // Hide the provider subcommand from help text
        if !FeatureFlag::ProviderCommand.is_enabled() {
            command = command.mut_subcommand("provider", |c| c.hide(true));
        }

        // Hide the integration subcommand from help text
        if !FeatureFlag::IntegrationCommand.is_enabled() {
            command = command.mut_subcommand("integration", |c| c.hide(true));
        }

        // Hide the schedule subcommand from help text.
        if !FeatureFlag::ScheduledAmbientAgents.is_enabled() {
            command = command.mut_subcommand("schedule", |c| c.hide(true));
        }

        // Hide the secret subcommand from help text.
        if !FeatureFlag::WarpManagedSecrets.is_enabled() {
            command = command.mut_subcommand("secret", |c| c.hide(true));
        }

        // Hide the federate subcommand from help text.
        if !FeatureFlag::OzIdentityFederation.is_enabled() {
            command = command.mut_subcommand("federate", |c| c.hide(true));
        }

        // Hide the harness-support subcommand from help text.
        if !FeatureFlag::AgentHarness.is_enabled() {
            command = command.mut_subcommand("harness-support", |c| c.hide(true));
        }

        // Hide the conversation subcommand and --conversation flag from help text.
        if !FeatureFlag::ConversationApi.is_enabled() {
            command = command.mut_subcommand("run", |run_cmd| {
                run_cmd
                    .mut_subcommand("conversation", |c| c.hide(true))
                    .mut_subcommand("get", |get_cmd| {
                        get_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
            });
        }
        // Hide the message subcommand from help text.
        if !FeatureFlag::OrchestrationV2.is_enabled() {
            command = command.mut_subcommand("run", |run_cmd| {
                run_cmd.mut_subcommand("message", |c| c.hide(true))
            });
        }

        // Hide the artifact subcommand from help text.
        if !FeatureFlag::ArtifactCommand.is_enabled() {
            command = command.mut_subcommand("artifact", |c| c.hide(true));
        }

        // Hide the api-key subcommand from help text.
        if !FeatureFlag::APIKeyManagement.is_enabled() {
            command = command.mut_subcommand("api-key", |c| c.hide(true));
        }

        // Wire up `--version` / `-V` using the same version metadata used elsewhere in the
        // app, so the CLI reports the build's release tag.
        command = command.version(version_string());

        // Substitute the actual binary name into help output. Ideally clap would do this for us.
        let bin_name =
            binary_name().unwrap_or_else(|| ChannelState::channel().cli_command_name().to_string());
        command =
            command.after_help(i18n::t("warp_cli.after_help").replace("{bin_name}", &bin_name));

        command
    }

    /// The requested subcommand, if any.
    pub fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }

    /// Args for the main Warp application, if not running a subcommand.
    pub fn app_args(&self) -> &AppArgs {
        &self.args
    }

    /// Extract the main Warp application args.
    pub fn into_app_args(self) -> AppArgs {
        self.args
    }

    /// Returns the global options.
    pub fn global_options(&self) -> &GlobalOptions {
        &self.global_options
    }

    /// Returns the API key if provided.
    pub fn api_key(&self) -> Option<&String> {
        self.global_options.api_key.as_ref()
    }

    /// Returns the output format.
    pub fn output_format(&self) -> OutputFormat {
        self.global_options.output_format
    }

    /// Returns true if debug logging is enabled.
    pub fn debug(&self) -> bool {
        self.debug
    }

    pub fn server_root_url(&self) -> Option<&str> {
        self.server_root_url.as_deref()
    }

    pub fn ws_server_url(&self) -> Option<&str> {
        self.ws_server_url.as_deref()
    }

    pub fn session_sharing_server_url(&self) -> Option<&str> {
        self.session_sharing_server_url.as_deref()
    }
}

fn localize_clap_command(mut command: clap::Command) -> clap::Command {
    command = command
        .about(i18n::t("warp_cli.about"))
        .mut_arg("api_key", |arg| {
            arg.help(i18n::t("warp_cli.arg.api_key.help"))
        })
        .mut_arg("output_format", |arg| {
            arg.help(i18n::t("warp_cli.arg.output_format.help"))
        })
        .mut_arg("debug", |arg| arg.help(i18n::t("warp_cli.arg.debug.help")));

    command = command
        .mut_subcommand("agent", |cmd| {
            localize_agent_command(cmd.about(i18n::t("warp_cli.command.agent.about")))
        })
        .mut_subcommand("environment", |cmd| {
            localize_environment_cli_command(
                cmd.about(i18n::t("warp_cli.command.environment.about")),
            )
        })
        .mut_subcommand("mcp", |cmd| {
            cmd.about(i18n::t("warp_cli.command.mcp.about"))
                .mut_subcommand("list", |cmd| {
                    cmd.about(i18n::t("warp_cli.command.mcp.list.about"))
                })
        })
        .mut_subcommand("run", |cmd| {
            localize_task_command(cmd.about(i18n::t("warp_cli.command.run.about")))
        })
        .mut_subcommand("model", |cmd| {
            localize_model_command(cmd.about(i18n::t("warp_cli.command.model.about")))
        })
        .mut_subcommand("login", |cmd| {
            cmd.about(i18n::t("warp_cli.command.login.about"))
        })
        .mut_subcommand("logout", |cmd| {
            cmd.about(i18n::t("warp_cli.command.logout.about"))
        })
        .mut_subcommand("whoami", |cmd| {
            cmd.about(i18n::t("warp_cli.command.whoami.about"))
        })
        .mut_subcommand("provider", |cmd| {
            localize_provider_command(cmd.about(i18n::t("warp_cli.command.provider.about")))
        })
        .mut_subcommand("integration", |cmd| {
            localize_integration_command(cmd.about(i18n::t("warp_cli.command.integration.about")))
        })
        .mut_subcommand("schedule", |cmd| {
            localize_schedule_command(
                cmd.about(i18n::t("warp_cli.command.schedule.about"))
                    .long_about(i18n::t("warp_cli.command.schedule.long_about")),
            )
        })
        .mut_subcommand("secret", |cmd| {
            localize_secret_command(cmd.about(i18n::t("warp_cli.command.secret.about")))
        })
        .mut_subcommand("federate", |cmd| {
            localize_federate_command(
                cmd.about(i18n::t("warp_cli.command.federate.about"))
                    .long_about(i18n::t("warp_cli.command.federate.long_about")),
            )
        })
        .mut_subcommand("artifact", |cmd| {
            localize_artifact_command(cmd.about(i18n::t("warp_cli.command.artifact.about")))
        })
        .mut_subcommand("api-key", |cmd| {
            localize_api_key_command(cmd.about(i18n::t("warp_cli.command.api_key.about")))
        })
        .mut_subcommand("completions", |cmd| {
            cmd.about(i18n::t("warp_cli.command.completions.about"))
                .long_about(i18n::t("warp_cli.command.completions.long_about"))
                .mut_arg("shell", |arg| {
                    arg.help(i18n::t("warp_cli.command.completions.shell.help"))
                })
        })
        .mut_subcommand("dump-debug-info", |cmd| {
            cmd.about(i18n::t("warp_cli.command.dump_debug_info.about"))
        })
        .mut_subcommand("harness-support", |cmd| {
            localize_harness_support_command(
                cmd.about(i18n::t("warp_cli.command.harness_support.about")),
            )
        })
        .mut_subcommand("minidump-server", |cmd| {
            cmd.about(i18n::t("warp_cli.command.worker.minidump_server.about"))
                .mut_arg("socket_name", |arg| {
                    arg.help(i18n::t("warp_cli.worker.arg.minidump_socket_name.help"))
                })
        });

    #[cfg(unix)]
    {
        command = command.mut_subcommand("terminal-server", |cmd| {
            cmd.about(i18n::t("warp_cli.command.worker.terminal_server.about"))
        });
    }

    #[cfg(feature = "plugin_host")]
    {
        command = command.mut_subcommand("plugin-host", |cmd| {
            cmd.about(i18n::t("warp_cli.command.worker.plugin_host.about"))
        });
    }

    #[cfg(not(target_family = "wasm"))]
    {
        command = command
            .mut_subcommand("remote-server-proxy", |cmd| {
                cmd.about(i18n::t("warp_cli.command.worker.remote_server_proxy.about"))
            })
            .mut_subcommand("remote-server-daemon", |cmd| {
                cmd.about(i18n::t(
                    "warp_cli.command.worker.remote_server_daemon.about",
                ))
            })
            .mut_subcommand("ripgrep-search", |cmd| {
                cmd.about(i18n::t("warp_cli.command.worker.ripgrep_search.about"))
                    .mut_arg("ignore_case", |arg| {
                        arg.help(i18n::t("warp_cli.worker.arg.ripgrep_ignore_case.help"))
                    })
                    .mut_arg("multiline", |arg| {
                        arg.help(i18n::t("warp_cli.worker.arg.ripgrep_multiline.help"))
                    })
                    .mut_arg("pattern", |arg| {
                        arg.help(i18n::t("warp_cli.worker.arg.ripgrep_pattern.help"))
                    })
                    .mut_arg("paths", |arg| {
                        arg.help(i18n::t("warp_cli.worker.arg.ripgrep_paths.help"))
                    })
            })
            .mut_subcommand("print-telemetry-events", |cmd| {
                cmd.about(i18n::t("warp_cli.command.print_telemetry_events.about"))
            });
    }

    command
}

fn localize_agent_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("run", |cmd| {
            localize_agent_run_args(cmd.about(i18n::t("warp_cli.command.agent.run.about")))
        })
        .mut_subcommand("run-cloud", |cmd| {
            localize_agent_run_cloud_args(
                cmd.about(i18n::t("warp_cli.command.agent.run_cloud.about")),
            )
        })
        .mut_subcommand("profile", |cmd| {
            cmd.about(i18n::t("warp_cli.command.agent.profile.about"))
                .mut_subcommand("list", |cmd| {
                    cmd.about(i18n::t("warp_cli.command.agent.profile.list.about"))
                })
        })
        .mut_subcommand("list", |cmd| {
            localize_agent_list_args(cmd.about(i18n::t("warp_cli.command.agent.list.about")))
        })
        .mut_subcommand("get", |cmd| {
            localize_agent_get_args(cmd.about(i18n::t("warp_cli.command.agent.get.about")))
        })
        .mut_subcommand("create", |cmd| {
            localize_agent_create_args(cmd.about(i18n::t("warp_cli.command.agent.create.about")))
        })
        .mut_subcommand("update", |cmd| {
            localize_agent_update_args(cmd.about(i18n::t("warp_cli.command.agent.update.about")))
        })
        .mut_subcommand("delete", |cmd| {
            localize_agent_delete_args(cmd.about(i18n::t("warp_cli.command.agent.delete.about")))
        })
        .mut_subcommand("skills", |cmd| {
            localize_agent_skills_args(cmd.about(i18n::t("warp_cli.command.agent.skills.about")))
        })
}

fn localize_agent_run_args(command: clap::Command) -> clap::Command {
    localize_snapshot_args(localize_config_file_args(localize_model_args(
        localize_prompt_args(command),
    )))
    .mut_arg("skill", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.skill.help"))
            .long_help(i18n::t("warp_cli.agent.arg.skill.long_help"))
    })
    .mut_arg("name", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.name.help"))
    })
    .mut_arg("cwd", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.cwd.help"))
    })
    .mut_arg("share", |arg| {
        arg.help(i18n::t("warp_cli.share.arg.help"))
            .long_help(i18n::t("warp_cli.share.arg.long_help"))
    })
    .mut_arg("mcp_specs", |arg| {
        arg.help(i18n::t("warp_cli.mcp.arg.spec.help"))
            .long_help(i18n::t("warp_cli.mcp.arg.spec.long_help"))
    })
    .mut_arg("environment", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.environment.help"))
    })
    .mut_arg("conversation", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.conversation.help"))
    })
    .mut_arg("profile", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.profile.help"))
    })
}

fn localize_agent_run_cloud_args(command: clap::Command) -> clap::Command {
    localize_snapshot_args(localize_computer_use_args(localize_scope_args(
        localize_environment_create_args(localize_config_file_args(localize_model_args(
            localize_prompt_args(command),
        ))),
    )))
    .mut_arg("skill", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.skill.help"))
            .long_help(i18n::t("warp_cli.agent.arg.skill.long_help"))
    })
    .mut_arg("name", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.name.help"))
    })
    .mut_arg("mcp_specs", |arg| {
        arg.help(i18n::t("warp_cli.mcp.arg.spec.help"))
            .long_help(i18n::t("warp_cli.mcp.arg.spec.long_help"))
    })
    .mut_arg("open", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.open.help"))
    })
    .mut_arg("conversation", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.conversation.help"))
    })
    .mut_arg("agent_uid", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.agent_uid.help"))
            .long_help(i18n::t("warp_cli.agent.arg.agent_uid.long_help"))
    })
    .mut_arg("worker_host", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.worker_host.help"))
            .long_help(i18n::t("warp_cli.agent.arg.worker_host.long_help"))
    })
    .mut_arg("attachment_paths", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.attachment_paths.help"))
            .long_help(i18n::t("warp_cli.agent.arg.attachment_paths.long_help"))
    })
}

fn localize_agent_list_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("sort_by", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.sort_by.help"))
        })
        .mut_arg("sort_order", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.sort_order.help"))
        })
}

fn localize_agent_get_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command).mut_arg("uid", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.uid.get.help"))
    })
}

fn localize_agent_create_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.name.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.description.help"))
        })
        .mut_arg("secrets", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.secrets.help"))
        })
        .mut_arg("skills", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.skills.help"))
        })
        .mut_arg("base_model", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.base_model.help"))
        })
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.create.environment.help"))
        })
}

fn localize_agent_update_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("uid", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.uid.update.help"))
        })
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.name.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.description.help"))
        })
        .mut_arg("remove_description", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_description.help"))
        })
        .mut_arg("add_secrets", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.add_secrets.help"))
        })
        .mut_arg("remove_secrets", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_secrets.help"))
        })
        .mut_arg("remove_all_secrets", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_all_secrets.help"))
        })
        .mut_arg("add_skills", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.add_skills.help"))
        })
        .mut_arg("remove_skills", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_skills.help"))
        })
        .mut_arg("remove_all_skills", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_all_skills.help"))
        })
        .mut_arg("base_model", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.base_model.help"))
        })
        .mut_arg("remove_base_model", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_base_model.help"))
        })
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.environment.help"))
        })
        .mut_arg("remove_environment", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.update.remove_environment.help"))
        })
}

fn localize_agent_delete_args(command: clap::Command) -> clap::Command {
    command.mut_arg("uid", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.uid.delete.help"))
    })
}

fn localize_agent_skills_args(command: clap::Command) -> clap::Command {
    command.mut_arg("repo", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.skills.repo.help"))
            .long_help(i18n::t("warp_cli.agent.arg.skills.repo.long_help"))
    })
}

fn localize_task_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("list", |cmd| {
            localize_task_list_args(cmd.about(i18n::t("warp_cli.command.run.list.about")))
        })
        .mut_subcommand("get", |cmd| {
            localize_task_get_args(cmd.about(i18n::t("warp_cli.command.run.get.about")))
        })
        .mut_subcommand("conversation", |cmd| {
            localize_conversation_command(
                cmd.about(i18n::t("warp_cli.command.run.conversation.about")),
            )
        })
        .mut_subcommand("message", |cmd| {
            localize_message_command(cmd.about(i18n::t("warp_cli.command.run.message.about")))
        })
}

fn localize_task_list_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("limit", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.limit.help"))
        })
        .mut_arg("state", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.state.help"))
        })
        .mut_arg("source", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.source.help"))
        })
        .mut_arg("execution_location", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.execution_location.help"))
        })
        .mut_arg("creator", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.creator.help"))
        })
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.environment.help"))
        })
        .mut_arg("skill", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.skill.help"))
        })
        .mut_arg("schedule", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.schedule.help"))
        })
        .mut_arg("ancestor_run", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.ancestor_run.help"))
        })
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.name.help"))
        })
        .mut_arg("model", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.model.help"))
        })
        .mut_arg("artifact_type", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.artifact_type.help"))
        })
        .mut_arg("created_after", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.created_after.help"))
        })
        .mut_arg("created_before", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.created_before.help"))
        })
        .mut_arg("updated_after", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.updated_after.help"))
        })
        .mut_arg("query", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.query.help"))
        })
        .mut_arg("sort_by", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.sort_by.help"))
        })
        .mut_arg("sort_order", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.sort_order.help"))
        })
        .mut_arg("cursor", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.cursor.help"))
                .long_help(i18n::t("warp_cli.task.arg.cursor.long_help"))
        })
}

fn localize_task_get_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("task_id", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.task_id.help"))
        })
        .mut_arg("conversation", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.conversation.help"))
        })
}

fn localize_conversation_command(command: clap::Command) -> clap::Command {
    command.mut_subcommand("get", |cmd| {
        cmd.about(i18n::t("warp_cli.command.run.conversation.get.about"))
            .mut_arg("conversation_id", |arg| {
                arg.help(i18n::t("warp_cli.task.arg.conversation_id.help"))
            })
    })
}

fn localize_message_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("watch", |cmd| {
            localize_message_watch_args(
                cmd.about(i18n::t("warp_cli.command.run.message.watch.about")),
            )
        })
        .mut_subcommand("send", |cmd| {
            localize_message_send_args(
                cmd.about(i18n::t("warp_cli.command.run.message.send.about")),
            )
        })
        .mut_subcommand("list", |cmd| {
            localize_message_list_args(
                cmd.about(i18n::t("warp_cli.command.run.message.list.about")),
            )
        })
        .mut_subcommand("read", |cmd| {
            localize_message_read_args(
                cmd.about(i18n::t("warp_cli.command.run.message.read.about")),
            )
        })
        .mut_subcommand("mark-delivered", |cmd| {
            localize_message_delivered_args(
                cmd.about(i18n::t("warp_cli.command.run.message.mark_delivered.about")),
            )
        })
}

fn localize_message_send_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("to", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.to.help"))
        })
        .mut_arg("subject", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.subject.help"))
        })
        .mut_arg("body", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.body.help"))
        })
        .mut_arg("sender_run_id", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.sender_run_id.help"))
        })
}

fn localize_message_list_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.run_id.list.help"))
        })
        .mut_arg("unread", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.unread.help"))
        })
        .mut_arg("since", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.since.help"))
        })
        .mut_arg("limit", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.limit.help"))
        })
}

fn localize_message_watch_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.run_id.watch.help"))
        })
        .mut_arg("since_sequence", |arg| {
            arg.help(i18n::t("warp_cli.task.arg.message.since_sequence.help"))
        })
}

fn localize_message_read_args(command: clap::Command) -> clap::Command {
    command.mut_arg("message_id", |arg| {
        arg.help(i18n::t("warp_cli.task.arg.message.message_id.read.help"))
    })
}

fn localize_message_delivered_args(command: clap::Command) -> clap::Command {
    command.mut_arg("message_id", |arg| {
        arg.help(i18n::t(
            "warp_cli.task.arg.message.message_id.mark_delivered.help",
        ))
    })
}

fn localize_schedule_command(command: clap::Command) -> clap::Command {
    localize_schedule_create_args(command)
        .mut_subcommand("create", |cmd| {
            localize_schedule_create_args(
                cmd.about(i18n::t("warp_cli.command.schedule.create.about")),
            )
        })
        .mut_subcommand("list", |cmd| {
            cmd.about(i18n::t("warp_cli.command.schedule.list.about"))
        })
        .mut_subcommand("get", |cmd| {
            localize_schedule_get_args(cmd.about(i18n::t("warp_cli.command.schedule.get.about")))
        })
        .mut_subcommand("update", |cmd| {
            localize_schedule_update_args(
                cmd.about(i18n::t("warp_cli.command.schedule.update.about")),
            )
        })
        .mut_subcommand("pause", |cmd| {
            localize_schedule_pause_args(
                cmd.about(i18n::t("warp_cli.command.schedule.pause.about"))
                    .long_about(i18n::t("warp_cli.command.schedule.pause.long_about")),
            )
        })
        .mut_subcommand("unpause", |cmd| {
            localize_schedule_unpause_args(
                cmd.about(i18n::t("warp_cli.command.schedule.unpause.about"))
                    .long_about(i18n::t("warp_cli.command.schedule.unpause.long_about")),
            )
        })
        .mut_subcommand("delete", |cmd| {
            localize_schedule_delete_args(
                cmd.about(i18n::t("warp_cli.command.schedule.delete.about")),
            )
        })
}

fn localize_schedule_create_args(command: clap::Command) -> clap::Command {
    localize_scope_args(localize_environment_create_args(localize_config_file_args(
        localize_model_args(command),
    )))
    .mut_arg("name", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.name.create.help"))
    })
    .mut_arg("cron", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.cron.create.help"))
    })
    .mut_arg("mcp_specs", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.mcp_specs.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.mcp_specs.long_help"))
    })
    .mut_arg("prompt", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.prompt.create.help"))
    })
    .mut_arg("skill", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.skill.create.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.skill.create.long_help"))
    })
    .mut_arg("worker_host", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.worker_host.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.worker_host.long_help"))
    })
}

fn localize_schedule_get_args(command: clap::Command) -> clap::Command {
    command.mut_arg("schedule_id", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.schedule_id.get.help"))
    })
}

fn localize_schedule_update_args(command: clap::Command) -> clap::Command {
    localize_schedule_environment_update_args(localize_config_file_args(localize_model_args(
        command,
    )))
    .mut_arg("schedule_id", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.schedule_id.update.help"))
    })
    .mut_arg("name", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.name.update.help"))
    })
    .mut_arg("cron", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.cron.update.help"))
    })
    .mut_arg("mcp_specs", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.mcp_specs.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.mcp_specs.long_help"))
    })
    .mut_arg("remove_mcp", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.remove_mcp.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.remove_mcp.long_help"))
    })
    .mut_arg("prompt", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.prompt.update.help"))
    })
    .mut_arg("skill", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.skill.update.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.skill.update.long_help"))
    })
    .mut_arg("remove_skill", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.remove_skill.help"))
    })
    .mut_arg("worker_host", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.worker_host.help"))
            .long_help(i18n::t("warp_cli.schedule.arg.worker_host.long_help"))
    })
}

fn localize_schedule_environment_update_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.schedule.arg.environment.update.help"))
        })
        .mut_arg("remove_environment", |arg| {
            arg.help(i18n::t("warp_cli.schedule.arg.remove_environment.help"))
        })
}

fn localize_schedule_pause_args(command: clap::Command) -> clap::Command {
    command.mut_arg("schedule_id", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.schedule_id.pause.help"))
    })
}

fn localize_schedule_unpause_args(command: clap::Command) -> clap::Command {
    command.mut_arg("schedule_id", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.schedule_id.unpause.help"))
    })
}

fn localize_schedule_delete_args(command: clap::Command) -> clap::Command {
    command.mut_arg("schedule_id", |arg| {
        arg.help(i18n::t("warp_cli.schedule.arg.schedule_id.delete.help"))
    })
}

fn localize_environment_cli_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("list", |cmd| {
            cmd.about(i18n::t("warp_cli.command.environment.list.about"))
        })
        .mut_subcommand("image", |cmd| {
            cmd.about(i18n::t("warp_cli.command.environment.image.about"))
                .mut_subcommand("list", |cmd| {
                    cmd.about(i18n::t("warp_cli.command.environment.image.list.about"))
                })
        })
        .mut_subcommand("create", |cmd| {
            localize_environment_create_command_args(
                cmd.about(i18n::t("warp_cli.command.environment.create.about")),
            )
        })
        .mut_subcommand("delete", |cmd| {
            localize_environment_delete_command_args(
                cmd.about(i18n::t("warp_cli.command.environment.delete.about")),
            )
        })
        .mut_subcommand("get", |cmd| {
            localize_environment_get_command_args(
                cmd.about(i18n::t("warp_cli.command.environment.get.about")),
            )
        })
        .mut_subcommand("update", |cmd| {
            localize_environment_update_command_args(
                cmd.about(i18n::t("warp_cli.command.environment.update.about")),
            )
        })
}

fn localize_environment_create_command_args(command: clap::Command) -> clap::Command {
    localize_scope_args(command)
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.name.create.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.description.create.help"))
        })
        .mut_arg("docker_image", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.docker_image.create.help"))
                .long_help(i18n::t(
                    "warp_cli.environment.arg.docker_image.create.long_help",
                ))
        })
        .mut_arg("repo", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.repo.create.help"))
        })
        .mut_arg("setup_command", |arg| {
            arg.help(i18n::t(
                "warp_cli.environment.arg.setup_command.create.help",
            ))
        })
}

fn localize_environment_delete_command_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("id", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.id.delete.help"))
        })
        .mut_arg("force", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.force.delete.help"))
        })
}

fn localize_environment_get_command_args(command: clap::Command) -> clap::Command {
    command.mut_arg("id", |arg| {
        arg.help(i18n::t("warp_cli.environment.arg.id.get.help"))
    })
}

fn localize_environment_update_command_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("id", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.id.update.help"))
        })
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.name.update.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.description.update.help"))
        })
        .mut_arg("remove_description", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.remove_description.help"))
        })
        .mut_arg("docker_image", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.docker_image.update.help"))
        })
        .mut_arg("repo", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.repo.update.help"))
        })
        .mut_arg("setup_command", |arg| {
            arg.help(i18n::t(
                "warp_cli.environment.arg.setup_command.update.help",
            ))
        })
        .mut_arg("remove_repo", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.remove_repo.help"))
        })
        .mut_arg("remove_setup_command", |arg| {
            arg.help(i18n::t(
                "warp_cli.environment.arg.remove_setup_command.help",
            ))
        })
        .mut_arg("force", |arg| {
            arg.help(i18n::t("warp_cli.environment.arg.force.update.help"))
        })
}

fn localize_secret_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("create", |cmd| {
            localize_secret_create_args(
                cmd.about(i18n::t("warp_cli.command.secret.create.about"))
                    .long_about(i18n::t("warp_cli.command.secret.create.long_about")),
            )
        })
        .mut_subcommand("delete", |cmd| {
            localize_secret_delete_args(cmd.about(i18n::t("warp_cli.command.secret.delete.about")))
        })
        .mut_subcommand("update", |cmd| {
            localize_secret_update_args(
                cmd.about(i18n::t("warp_cli.command.secret.update.about"))
                    .long_about(i18n::t("warp_cli.command.secret.update.long_about")),
            )
        })
        .mut_subcommand("list", |cmd| {
            cmd.about(i18n::t("warp_cli.command.secret.list.about"))
        })
}

fn localize_secret_create_args(command: clap::Command) -> clap::Command {
    localize_secret_value_args(localize_common_secret_create_args(command))
        .mut_arg("secret_type", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.secret_type.help"))
        })
        .mut_subcommand("claude", |cmd| {
            localize_secret_claude_create_command(
                cmd.about(i18n::t("warp_cli.command.secret.create.claude.about")),
            )
        })
        .mut_subcommand("codex", |cmd| {
            localize_secret_codex_create_command(
                cmd.about(i18n::t("warp_cli.command.secret.create.codex.about")),
            )
        })
}

fn localize_secret_claude_create_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("api-key", |cmd| {
            localize_anthropic_api_key_args(cmd.about(i18n::t(
                "warp_cli.command.secret.create.claude.api_key.about",
            )))
        })
        .mut_subcommand("bedrock-api-key", |cmd| {
            localize_bedrock_api_key_args(cmd.about(i18n::t(
                "warp_cli.command.secret.create.claude.bedrock_api_key.about",
            )))
        })
        .mut_subcommand("bedrock-access-key", |cmd| {
            localize_bedrock_access_key_args(cmd.about(i18n::t(
                "warp_cli.command.secret.create.claude.bedrock_access_key.about",
            )))
        })
}

fn localize_secret_codex_create_command(command: clap::Command) -> clap::Command {
    command.mut_subcommand("api-key", |cmd| {
        localize_openai_api_key_args(cmd.about(i18n::t(
            "warp_cli.command.secret.create.codex.api_key.about",
        )))
    })
}

fn localize_common_secret_create_args(command: clap::Command) -> clap::Command {
    localize_scope_args(command)
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.name.create.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.description.help"))
        })
}

fn localize_secret_value_args(command: clap::Command) -> clap::Command {
    command.mut_arg("value_file", |arg| {
        arg.help(i18n::t("warp_cli.secret.arg.value_file.help"))
            .long_help(i18n::t("warp_cli.secret.arg.value_file.long_help"))
    })
}

fn localize_anthropic_api_key_args(command: clap::Command) -> clap::Command {
    localize_secret_value_args(localize_common_secret_create_args(command))
}

fn localize_bedrock_api_key_args(command: clap::Command) -> clap::Command {
    localize_common_secret_create_args(command)
        .mut_arg("bedrock_api_key", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.bedrock_api_key.help"))
        })
        .mut_arg("region", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.bedrock_region.help"))
        })
}

fn localize_bedrock_access_key_args(command: clap::Command) -> clap::Command {
    localize_common_secret_create_args(command)
        .mut_arg("access_key_id", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.aws_access_key_id.help"))
        })
        .mut_arg("secret_access_key", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.aws_secret_access_key.help"))
        })
        .mut_arg("session_token", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.aws_session_token.help"))
        })
        .mut_arg("region", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.bedrock_region.help"))
        })
}

fn localize_openai_api_key_args(command: clap::Command) -> clap::Command {
    localize_secret_value_args(localize_common_secret_create_args(command)).mut_arg(
        "base_url",
        |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.openai_base_url.help"))
                .long_help(i18n::t("warp_cli.secret.arg.openai_base_url.long_help"))
        },
    )
}

fn localize_secret_delete_args(command: clap::Command) -> clap::Command {
    localize_scope_args(command)
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.name.delete.help"))
        })
        .mut_arg("force", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.force.delete.help"))
        })
}

fn localize_secret_update_args(command: clap::Command) -> clap::Command {
    localize_secret_value_args(localize_scope_args(command))
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.name.update.help"))
        })
        .mut_arg("value", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.value.update.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.secret.arg.description.update.help"))
        })
}

fn localize_integration_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("create", |cmd| {
            localize_integration_create_args(
                cmd.about(i18n::t("warp_cli.command.integration.create.about")),
            )
        })
        .mut_subcommand("update", |cmd| {
            localize_integration_update_args(
                cmd.about(i18n::t("warp_cli.command.integration.update.about")),
            )
        })
        .mut_subcommand("list", |cmd| {
            cmd.about(i18n::t("warp_cli.command.integration.list.about"))
        })
}

fn localize_integration_create_args(command: clap::Command) -> clap::Command {
    localize_environment_create_args(localize_config_file_args(localize_model_args(command)))
        .mut_arg("provider", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.provider.create.help"))
        })
        .mut_arg("mcp_specs", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.mcp_specs.help"))
                .long_help(i18n::t("warp_cli.integration.arg.mcp_specs.long_help"))
        })
        .mut_arg("prompt", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.prompt.help"))
        })
        .mut_arg("worker_host", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.worker_host.help"))
                .long_help(i18n::t("warp_cli.integration.arg.worker_host.long_help"))
        })
}

fn localize_integration_update_args(command: clap::Command) -> clap::Command {
    localize_integration_environment_update_args(localize_config_file_args(localize_model_args(
        command,
    )))
    .mut_arg("provider", |arg| {
        arg.help(i18n::t("warp_cli.integration.arg.provider.update.help"))
    })
    .mut_arg("mcp_specs", |arg| {
        arg.help(i18n::t("warp_cli.integration.arg.mcp_specs.help"))
            .long_help(i18n::t("warp_cli.integration.arg.mcp_specs.long_help"))
    })
    .mut_arg("remove_mcp", |arg| {
        arg.help(i18n::t("warp_cli.integration.arg.remove_mcp.help"))
            .long_help(i18n::t("warp_cli.integration.arg.remove_mcp.long_help"))
    })
    .mut_arg("prompt", |arg| {
        arg.help(i18n::t("warp_cli.integration.arg.prompt.help"))
    })
    .mut_arg("worker_host", |arg| {
        arg.help(i18n::t("warp_cli.integration.arg.worker_host.help"))
            .long_help(i18n::t("warp_cli.integration.arg.worker_host.long_help"))
    })
}

fn localize_integration_environment_update_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.environment.update.help"))
        })
        .mut_arg("remove_environment", |arg| {
            arg.help(i18n::t("warp_cli.integration.arg.remove_environment.help"))
        })
}

fn localize_api_key_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("list", |cmd| {
            localize_api_key_list_args(cmd.about(i18n::t("warp_cli.command.api_key.list.about")))
        })
        .mut_subcommand("create", |cmd| {
            localize_api_key_create_args(
                cmd.about(i18n::t("warp_cli.command.api_key.create.about")),
            )
        })
        .mut_subcommand("expire", |cmd| {
            localize_api_key_expire_args(
                cmd.about(i18n::t("warp_cli.command.api_key.expire.about")),
            )
        })
}

fn localize_api_key_list_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("sort_by", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.sort_by.help"))
        })
        .mut_arg("sort_order", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.sort_order.help"))
        })
}

fn localize_api_key_create_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("name", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.name.create.help"))
        })
        .mut_arg("agent_uid", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.agent_uid.help"))
        })
        .mut_arg("expires_in", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.expires_in.help"))
        })
        .mut_arg("expires_at", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.expires_at.help"))
        })
        .mut_arg("no_expiration", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.no_expiration.help"))
        })
}

fn localize_api_key_expire_args(command: clap::Command) -> clap::Command {
    localize_json_output_args(command)
        .mut_arg("key_uid", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.key_uid.expire.help"))
        })
        .mut_arg("force", |arg| {
            arg.help(i18n::t("warp_cli.api_key.arg.force.expire.help"))
        })
}

fn localize_artifact_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("upload", |cmd| {
            localize_artifact_upload_args(
                cmd.about(i18n::t("warp_cli.command.artifact.upload.about")),
            )
        })
        .mut_subcommand("get", |cmd| {
            localize_artifact_get_args(cmd.about(i18n::t("warp_cli.command.artifact.get.about")))
        })
        .mut_subcommand("download", |cmd| {
            localize_artifact_download_args(
                cmd.about(i18n::t("warp_cli.command.artifact.download.about")),
            )
        })
}

fn localize_artifact_upload_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("path", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.path.upload.help"))
        })
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.run_id.help"))
        })
        .mut_arg("conversation_id", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.conversation_id.help"))
        })
        .mut_arg("description", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.description.help"))
        })
}

fn localize_artifact_get_args(command: clap::Command) -> clap::Command {
    command.mut_arg("artifact_uid", |arg| {
        arg.help(i18n::t("warp_cli.artifact.arg.artifact_uid.get.help"))
    })
}

fn localize_artifact_download_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("artifact_uid", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.artifact_uid.download.help"))
        })
        .mut_arg("out", |arg| {
            arg.help(i18n::t("warp_cli.artifact.arg.out.help"))
        })
}

fn localize_federate_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("issue-token", |cmd| {
            localize_federate_issue_token_args(
                cmd.about(i18n::t("warp_cli.command.federate.issue_token.about")),
            )
        })
        .mut_subcommand("issue-gcp-token", |cmd| {
            localize_federate_issue_gcp_token_args(
                cmd.about(i18n::t("warp_cli.command.federate.issue_gcp_token.about"))
                    .long_about(i18n::t(
                        "warp_cli.command.federate.issue_gcp_token.long_about",
                    )),
            )
        })
}

fn localize_federate_issue_token_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.run_id.help"))
        })
        .mut_arg("audience", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.audience.help"))
        })
        .mut_arg("duration", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.duration.help"))
        })
        .mut_arg("subject_template", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.subject_template.help"))
                .long_help(i18n::t("warp_cli.federate.arg.subject_template.long_help"))
        })
}

fn localize_federate_issue_gcp_token_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.run_id.help"))
        })
        .mut_arg("duration", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.duration.help"))
        })
        .mut_arg("audience", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.gcp_audience.help"))
        })
        .mut_arg("token_type", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.gcp_token_type.help"))
        })
        .mut_arg("output_file", |arg| {
            arg.help(i18n::t("warp_cli.federate.arg.gcp_output_file.help"))
        })
}

fn localize_provider_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("setup", |cmd| {
            localize_provider_setup_args(
                cmd.about(i18n::t("warp_cli.command.provider.setup.about")),
            )
        })
        .mut_subcommand("list", |cmd| {
            cmd.about(i18n::t("warp_cli.command.provider.list.about"))
        })
}

fn localize_provider_setup_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("provider_type", |arg| {
            arg.help(i18n::t("warp_cli.provider.arg.provider_type.help"))
        })
        .mut_arg("team", |arg| {
            arg.help(i18n::t("warp_cli.provider.arg.team.help"))
        })
        .mut_arg("personal", |arg| {
            arg.help(i18n::t("warp_cli.provider.arg.personal.help"))
        })
}

fn localize_model_command(command: clap::Command) -> clap::Command {
    command.mut_subcommand("list", |cmd| {
        cmd.about(i18n::t("warp_cli.command.model.list.about"))
    })
}

fn localize_harness_support_command(command: clap::Command) -> clap::Command {
    command
        .mut_arg("run_id", |arg| {
            arg.help(i18n::t("warp_cli.harness_support.arg.run_id.help"))
        })
        .mut_subcommand("ping", |cmd| {
            cmd.about(i18n::t("warp_cli.command.harness_support.ping.about"))
        })
        .mut_subcommand("report-artifact", |cmd| {
            cmd.about(i18n::t(
                "warp_cli.command.harness_support.report_artifact.about",
            ))
            .mut_subcommand("pull-request", |cmd| {
                cmd.about(i18n::t(
                    "warp_cli.command.harness_support.report_artifact.pull_request.about",
                ))
                .mut_arg("url", |arg| {
                    arg.help(i18n::t(
                        "warp_cli.harness_support.arg.pull_request.url.help",
                    ))
                })
                .mut_arg("branch", |arg| {
                    arg.help(i18n::t(
                        "warp_cli.harness_support.arg.pull_request.branch.help",
                    ))
                })
            })
        })
        .mut_subcommand("notify-user", |cmd| {
            cmd.about(i18n::t(
                "warp_cli.command.harness_support.notify_user.about",
            ))
            .mut_arg("message", |arg| {
                arg.help(i18n::t("warp_cli.harness_support.arg.message.help"))
            })
        })
        .mut_subcommand("finish-task", |cmd| {
            cmd.about(i18n::t(
                "warp_cli.command.harness_support.finish_task.about",
            ))
            .mut_arg("status", |arg| {
                arg.help(i18n::t("warp_cli.harness_support.arg.status.help"))
            })
            .mut_arg("summary", |arg| {
                arg.help(i18n::t("warp_cli.harness_support.arg.summary.help"))
            })
        })
        .mut_subcommand("report-shutdown", |cmd| {
            cmd.about(i18n::t(
                "warp_cli.command.harness_support.report_shutdown.about",
            ))
            .mut_arg("error_category", |arg| {
                arg.help(i18n::t("warp_cli.harness_support.arg.error_category.help"))
            })
            .mut_arg("error_message", |arg| {
                arg.help(i18n::t("warp_cli.harness_support.arg.error_message.help"))
            })
        })
}

fn localize_prompt_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("prompt", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.prompt.help"))
        })
        .mut_arg("saved_prompt", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.saved_prompt.help"))
        })
}

fn localize_model_args(command: clap::Command) -> clap::Command {
    command.mut_arg("model", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.model.help"))
    })
}

fn localize_config_file_args(command: clap::Command) -> clap::Command {
    command.mut_arg("file", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.config_file.help"))
    })
}

fn localize_json_output_args(command: clap::Command) -> clap::Command {
    command.mut_arg("filter", |arg| {
        arg.help(i18n::t("warp_cli.agent.arg.jq_filter.help"))
            .long_help(i18n::t("warp_cli.agent.arg.jq_filter.long_help"))
    })
}

fn localize_snapshot_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("no_snapshot", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.no_snapshot.help"))
        })
        .mut_arg("snapshot_upload_timeout", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.snapshot_upload_timeout.help"))
        })
        .mut_arg("snapshot_script_timeout", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.snapshot_script_timeout.help"))
        })
}

fn localize_scope_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("team", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.scope.team.help"))
        })
        .mut_arg("personal", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.scope.personal.help"))
        })
}

fn localize_environment_create_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("environment", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.environment.help"))
        })
        .mut_arg("no_environment", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.no_environment.help"))
        })
}

fn localize_computer_use_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("computer_use", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.computer_use.help"))
        })
        .mut_arg("no_computer_use", |arg| {
            arg.help(i18n::t("warp_cli.agent.arg.no_computer_use.help"))
        })
}

fn exit_unrecognized_subcommand(subcommand: &str) -> ! {
    eprintln!(
        "{}",
        i18n::t("warp_cli.error.unrecognized_subcommand").replace("{subcommand}", subcommand)
    );
    eprintln!();
    eprintln!("{}", i18n::t("warp_cli.error.more_info_help"));
    std::process::exit(2);
}

/// Warp may spawn several worker processes - mostly servers that support the main application.
///
/// These subcommands run those worker processes, which are bundled into the Warp binary.
#[derive(Debug, Clone, Subcommand)]
pub enum WorkerCommand {
    /// Run the terminal server.
    #[clap(hide = true)]
    #[cfg(unix)]
    TerminalServer(TerminalServerArgs),

    /// Run this process as the plugin host rather than the main app.
    #[cfg(feature = "plugin_host")]
    #[clap(long_flag = "plugin-host")]
    PluginHost {
        #[clap(flatten)]
        parent: ParentOpts,
    },

    /// Run the minidump server.
    #[clap(hide = true)]
    MinidumpServer {
        /// Socket name for the minidump server.
        socket_name: std::path::PathBuf,
    },

    /// Run the remote development server proxy over SSH stdio.
    /// Ensures the daemon is running, then bridges its stdin/stdout
    /// to the daemon via a Unix domain socket.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RemoteServerProxy(RemoteServerIdentityArgs),

    /// Run the long-lived remote development server daemon.
    /// Listens on a Unix domain socket and accepts multiple concurrent
    /// connections from proxy processes.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RemoteServerDaemon(RemoteServerIdentityArgs),

    /// Run a headless ripgrep search worker.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RipgrepSearch {
        #[clap(flatten)]
        parent: ParentOpts,
        #[clap(long = "ignore-case")]
        ignore_case: bool,
        #[clap(long = "multiline")]
        multiline: bool,
        /// Search pattern.
        pattern: String,
        /// Paths to search.
        paths: Vec<std::path::PathBuf>,
    },
}

/// CLI-related subcommands. The command-line interface to Warp isn't a full SDK (e.g. with language bindings),
/// but it allows scripting some Warp functionality.
#[derive(Debug, Clone, Subcommand)]
pub enum CliCommand {
    /// Interact with Oz.
    #[command(subcommand)]
    Agent(crate::agent::AgentCommand),

    /// Manage cloud environments.
    #[command(subcommand)]
    Environment(crate::environment::EnvironmentCommand),

    /// Manage MCP servers.
    #[command(subcommand)]
    MCP(crate::mcp::MCPCommand),

    /// Manage runs.
    #[command(subcommand, alias = "task")]
    Run(crate::task::TaskCommand),

    /// Manage available models.
    #[command(subcommand)]
    Model(crate::model::ModelCommand),

    /// Log in to Warp.
    Login,
    /// Log out of Warp.
    Logout,
    /// Print information about the logged-in user.
    Whoami,

    /// Manage providers.
    #[command(subcommand)]
    Provider(crate::provider::ProviderCommand),

    /// Manage integrations.
    #[command(subcommand)]
    Integration(crate::integration::IntegrationCommand),

    /// Create and manage scheduled Oz agents. Scheduled agents run a user-defined task periodically, according to a cron schedule.
    ///
    /// As a shorthand, the `schedule` command behaves identically to `schedule create`.
    Schedule(crate::schedule::ScheduleCommand),

    /// Manage secrets.
    #[command(subcommand)]
    Secret(crate::secret::SecretCommand),

    /// Issue and manage federated identity tokens.
    #[command(subcommand)]
    Federate(crate::federate::FederateCommand),

    /// Support commands for agent harnesses to integrate with Oz.
    #[command(hide = true)]
    HarnessSupport(crate::harness_support::HarnessSupportArgs),

    /// Manage artifacts.
    #[command(subcommand)]
    Artifact(crate::artifact::ArtifactCommand),

    /// Manage API keys.
    #[command(subcommand)]
    ApiKey(crate::api_key::ApiKeyCommand),
}

/// A subcommand of the main Warp application. This includes all [`WorkerCommand`]s as well as app-specific debugging tools.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[clap(flatten)]
    Worker(WorkerCommand),

    /// Commands that make up the Warp CLI.
    #[clap(flatten)]
    CommandLine(Box<CliCommand>),

    /// Generate shell completions for your shell to stdout.
    ///
    ///
    /// For bash, add the following to ~/.bashrc:
    ///     source <(path/to/warp completions bash)
    ///
    /// For zsh, add the following to ~/.zshrc:
    ///     source <(path/to/warp completions zsh)
    ///
    /// For fish, add the following to ~/.config/fish/config.fish:
    ///     path/to/warp completions fish | source
    ///
    /// For Powershell, add the following to $PROFILE:
    ///     path\to\warp | Out-String | Invoke-Expression
    ///
    /// If no shell is provided, this defaults to the shell that Warp was run from.
    #[command(verbatim_doc_comment)]
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Option<clap_complete::aot::Shell>,
    },

    /// Print debugging information and exit.
    #[clap(long_flag = "dump-debug-info")]
    DumpDebugInfo,

    /// Print telemetry events in production and exit.
    #[clap(long_flag = "print-telemetry-events", hide = true)]
    #[cfg(not(target_family = "wasm"))]
    PrintTelemetryEvents,
}

impl Command {
    /// Whether or not the Command should print to stdout.
    pub fn prints_to_stdout(&self) -> bool {
        match self {
            Command::Worker(_) => false,
            Command::CommandLine(_) | Command::DumpDebugInfo => true,
            Command::Completions { .. } => true,
            #[cfg(not(target_family = "wasm"))]
            Command::PrintTelemetryEvents => true,
        }
    }
}

/// Arguments for the terminal server.
#[cfg(not(windows))]
#[derive(Debug, Clone, Default, clap::Args)]
pub struct TerminalServerArgs {
    #[clap(flatten)]
    pub parent: ParentOpts,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum RecoveryMechanism {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[value(name = "force-x11")]
    X11,
    #[value(name = "force-dedicated-gpu")]
    DedicatedGpu,
    #[value(name = "disable-opengl")]
    DisableOpenGL,
    #[value(name = "force-vulkan")]
    ForceVulkan,
}

impl fmt::Display for RecoveryMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no values are skipped");
        f.write_str(value.get_name())
    }
}

/// Returns the subcommand name to use for starting the terminal server.
pub fn terminal_server_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("terminal-server")
        .expect("terminal-server subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the subcommand name to use for starting the installation detection server.
pub fn installation_detection_server_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("installation-detection-server")
        .expect("installation-detection-server subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the subcommand name to use for starting the ripgrep search worker.
#[cfg(not(target_family = "wasm"))]
pub fn ripgrep_search_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("ripgrep-search")
        .expect("ripgrep-search subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the flag to use when finishing the auto-update process.
pub fn finish_update_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .get_arguments()
        .find(|arg| arg.get_long() == Some("finish-update"))
        .expect("finish-update flag not found")
        .get_long()
        .unwrap();
    format!("--{flag}")
}

/// Returns the flag to use for the dump-debug-info subcommand.
pub fn dump_debug_info_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .find_subcommand("dump-debug-info")
        .expect("dump-debug-info subcommand not found")
        .get_long_flag()
        .expect("dump-debug-info flag not found");
    format!("--{flag}")
}

/// Returns a flag that sets the current process as the parent of a Warp subcommand to spawn.
pub fn parent_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .get_arguments()
        .find(|arg| arg.get_long() == Some("parent-pid"))
        .expect("parent-pid flag not found")
        .get_long()
        .unwrap();
    format!("--{flag}={}", std::process::id())
}

/// The name that this binary was invoked as.
pub fn binary_name() -> Option<String> {
    // Adapted from https://github.com/clap-rs/clap/blob/2c04acd3607e5c4676477ca14948419bb31c73a1/clap_builder/src/builder/command.rs#L888-L902
    // Unfortunately, we can't use Command::get_bin_name because it's not populated until args are parsed.
    let arg0 = env::args().next()?;
    Path::new(&arg0).file_name()?.to_str().map(|s| s.to_owned())
}

/// The version string shown for `--version` / `-V`.
///
/// Sourced from [`ChannelState::app_version`], which is populated from the
/// `GIT_RELEASE_TAG` env var at compile time. Falls back to a placeholder for
/// untagged builds (e.g. local `cargo run`).
pub fn version_string() -> &'static str {
    ChannelState::app_version().unwrap_or("<unknown>")
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
