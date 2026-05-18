//! Commands to manage named agents via the public API.

use std::cmp::Ordering;
use std::io::Write as _;

use anyhow::anyhow;
use comfy_table::Cell;
use serde::Serialize;
use warp_cli::agent::{
    AgentCreateArgs, AgentDeleteArgs, AgentGetArgs, AgentListArgs, AgentSortByArg,
    AgentSortOrderArg, AgentUpdateArgs, OutputFormat,
};
use warp_cli::json_filter::JsonOutput;
use warpui::{platform::TerminationMode, AppContext, ModelContext, SingletonEntity};

use crate::server::server_api::ai::{
    AgentResponse, CreateAgentRequest, SecretRef, UpdateAgentRequest,
};
use crate::server::server_api::ServerApiProvider;

use super::output::TableFormat;

/// Singleton model that runs async work for named-agent CLI commands.
struct AgentManagementRunner;

pub fn list_agents(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: AgentListArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentManagementRunner);
    runner.update(ctx, |runner, ctx| runner.list(args, output_format, ctx))
}

pub fn get_agent(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: AgentGetArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentManagementRunner);
    runner.update(ctx, |runner, ctx| runner.get(args, output_format, ctx))
}

pub fn create_agent(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: AgentCreateArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentManagementRunner);
    runner.update(ctx, |runner, ctx| runner.create(args, output_format, ctx))
}

pub fn update_agent(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: AgentUpdateArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentManagementRunner);
    runner.update(ctx, |runner, ctx| runner.update(args, output_format, ctx))
}

pub fn delete_agent(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: AgentDeleteArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentManagementRunner);
    runner.update(ctx, |runner, ctx| runner.delete(args, output_format, ctx))
}
impl AgentManagementRunner {
    fn spawn_command(
        &self,
        future: impl warpui::r#async::Spawnable<Output = anyhow::Result<()>>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(future, |_, result, ctx| match result {
            Ok(()) => {
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                super::report_fatal_error(err, ctx);
            }
        });
    }

    fn list(
        &self,
        args: AgentListArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let future = async move {
            ensure_json_sort_is_not_requested(output_format, &args.json_output, &args)?;

            if matches!(output_format, OutputFormat::Json) || args.json_output.force_json_output() {
                let response = ai_client.list_agents_raw().await?;
                super::output::print_raw_json(response, &args.json_output)?;
            } else {
                let mut agents = ai_client.list_agents().await?;
                sort_agents(&mut agents, args.sort_by, args.sort_order);
                print_agents(&agents, output_format)?;
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn get(
        &self,
        args: AgentGetArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let future = async move {
            if matches!(output_format, OutputFormat::Json) || args.json_output.force_json_output() {
                let response = ai_client.get_agent_raw(&args.uid).await?;
                super::output::print_raw_json(response, &args.json_output)?;
            } else {
                let agent = ai_client.get_agent(&args.uid).await?;
                print_single_agent(&agent, output_format)?;
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn create(
        &self,
        args: AgentCreateArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let future = async move {
            let json_output = args.json_output.clone();
            let request = CreateAgentRequest {
                name: args.name,
                description: args.description,
                secrets: secret_refs(args.secrets),
                skills: args.skills,
                base_model: args.base_model,
                environment_id: args.environment,
            };
            if matches!(output_format, OutputFormat::Json) || json_output.force_json_output() {
                let response = ai_client.create_agent_raw(request).await?;
                super::output::print_raw_json(response, &json_output)?;
            } else {
                let agent = ai_client.create_agent(request).await?;
                print_single_agent(&agent, output_format)?;
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn update(
        &self,
        args: AgentUpdateArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let future = async move {
            let uid = args.uid.clone();
            let json_output = args.json_output.clone();
            let current_agent = if args.add_secrets.is_empty()
                && args.remove_secrets.is_empty()
                && args.add_skills.is_empty()
                && args.remove_skills.is_empty()
            {
                None
            } else {
                Some(ai_client.get_agent(&uid).await?)
            };
            let request = UpdateAgentRequest {
                name: args.name,
                description: if args.remove_description {
                    Some(String::new())
                } else {
                    args.description
                },
                secrets: if args.remove_all_secrets {
                    Some(vec![])
                } else if args.add_secrets.is_empty() && args.remove_secrets.is_empty() {
                    None
                } else {
                    let current_agent = current_agent
                        .as_ref()
                        .expect("current agent is fetched when applying secret deltas");
                    Some(apply_secret_deltas(
                        &current_agent.secrets,
                        args.add_secrets,
                        args.remove_secrets,
                    ))
                },
                skills: if args.remove_all_skills {
                    Some(vec![])
                } else if args.add_skills.is_empty() && args.remove_skills.is_empty() {
                    None
                } else {
                    let current_agent = current_agent
                        .as_ref()
                        .expect("current agent is fetched when applying skill deltas");
                    Some(apply_string_deltas(
                        &current_agent.skills,
                        args.add_skills,
                        args.remove_skills,
                    ))
                },
                base_model: if args.remove_base_model {
                    Some(String::new())
                } else {
                    args.base_model
                },
                environment_id: if args.remove_environment {
                    Some(String::new())
                } else {
                    args.environment
                },
            };
            if request_is_empty(&request) {
                return Err(anyhow!("No updates requested"));
            }

            if matches!(output_format, OutputFormat::Json) || json_output.force_json_output() {
                let response = ai_client.update_agent_raw(&uid, request).await?;
                super::output::print_raw_json(response, &json_output)?;
            } else {
                let agent = ai_client.update_agent(&uid, request).await?;
                print_single_agent(&agent, output_format)?;
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn delete(
        &self,
        args: AgentDeleteArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let future = async move {
            ai_client.delete_agent(&args.uid).await?;
            print_delete_result(&args.uid, output_format)?;
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }
}

fn secret_refs(secrets: Vec<String>) -> Vec<SecretRef> {
    secrets.into_iter().map(|name| SecretRef { name }).collect()
}
fn apply_secret_deltas(
    current: &[SecretRef],
    add_secrets: Vec<String>,
    remove_secrets: Vec<String>,
) -> Vec<SecretRef> {
    let names = apply_string_deltas(
        &current
            .iter()
            .map(|secret| secret.name.clone())
            .collect::<Vec<_>>(),
        add_secrets,
        remove_secrets,
    );
    secret_refs(names)
}

fn apply_string_deltas(
    current: &[String],
    add_values: Vec<String>,
    remove_values: Vec<String>,
) -> Vec<String> {
    let mut values = current
        .iter()
        .filter(|value| !remove_values.contains(value))
        .cloned()
        .collect::<Vec<_>>();
    for value in add_values {
        if !values.contains(&value) {
            values.push(value);
        }
    }
    values
}

fn request_is_empty(request: &UpdateAgentRequest) -> bool {
    request.name.is_none()
        && request.description.is_none()
        && request.secrets.is_none()
        && request.skills.is_none()
        && request.base_model.is_none()
        && request.environment_id.is_none()
}

fn ensure_json_sort_is_not_requested(
    output_format: OutputFormat,
    json_output: &JsonOutput,
    args: &AgentListArgs,
) -> anyhow::Result<()> {
    if (matches!(output_format, OutputFormat::Json) || json_output.force_json_output())
        && (args.sort_by.is_some() || args.sort_order.is_some())
    {
        return Err(anyhow!(
            "--sort-by and --sort-order are not supported with JSON output"
        ));
    }

    Ok(())
}

fn sort_agents(
    agents: &mut [AgentResponse],
    sort_by: Option<AgentSortByArg>,
    sort_order: Option<AgentSortOrderArg>,
) {
    let sort_by = sort_by.unwrap_or(AgentSortByArg::Name);
    let default_order = match sort_by {
        AgentSortByArg::Name => AgentSortOrderArg::Asc,
        AgentSortByArg::CreatedAt => AgentSortOrderArg::Desc,
    };
    let sort_order = sort_order.unwrap_or(default_order);

    agents.sort_by(|left, right| {
        let ordering = match sort_by {
            AgentSortByArg::Name => left
                .name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.uid.cmp(&right.uid)),
            AgentSortByArg::CreatedAt => left
                .created_at
                .cmp(&right.created_at)
                .then_with(|| left.uid.cmp(&right.uid)),
        };

        match sort_order {
            AgentSortOrderArg::Asc => ordering,
            AgentSortOrderArg::Desc => match ordering {
                Ordering::Less => Ordering::Greater,
                Ordering::Equal => Ordering::Equal,
                Ordering::Greater => Ordering::Less,
            },
        }
    });
}

impl TableFormat for AgentResponse {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UID"),
            Cell::new("Name"),
            Cell::new("Available"),
            Cell::new("Created"),
            Cell::new("Description"),
            Cell::new("Secrets"),
            Cell::new("Skills"),
            Cell::new("Base model"),
            Cell::new("Environment"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.name),
            Cell::new(self.available.to_string()),
            Cell::new(self.created_at.to_rfc3339()),
            Cell::new(display_optional(self.description.as_deref())),
            Cell::new(display_list(
                self.secrets.iter().map(|secret| secret.name.as_str()),
            )),
            Cell::new(display_list(self.skills.iter().map(String::as_str))),
            Cell::new(display_optional(self.base_model.as_deref())),
            Cell::new(display_optional(self.environment_id.as_deref())),
        ]
    }
}

fn print_agents(agents: &[AgentResponse], output_format: OutputFormat) -> anyhow::Result<()> {
    match output_format {
        OutputFormat::Pretty if agents.is_empty() => {
            println!("No agents found.");
            print_skills_hint();
        }
        OutputFormat::Pretty => {
            super::output::write_list(agents.to_vec(), output_format, std::io::stdout())?;
            print_skills_hint();
        }
        OutputFormat::Text => {
            super::output::write_list(agents.to_vec(), output_format, std::io::stdout())?;
        }
        OutputFormat::Ndjson => {
            for agent in agents {
                super::output::write_json_line(agent, std::io::stdout())?;
            }
        }
        OutputFormat::Json => unreachable!("JSON output is handled by the raw API path"),
    }
    Ok(())
}

fn print_single_agent(agent: &AgentResponse, output_format: OutputFormat) -> anyhow::Result<()> {
    match output_format {
        OutputFormat::Pretty | OutputFormat::Text => {
            super::output::write_list([agent.clone()], output_format, std::io::stdout())?;
        }
        OutputFormat::Ndjson => {
            super::output::write_json_line(agent, std::io::stdout())?;
        }
        OutputFormat::Json => unreachable!("JSON output is handled by the raw API path"),
    }
    Ok(())
}

fn print_skills_hint() {
    let binary_name = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());
    println!("Looking for agent skills? Use {binary_name} agent skills instead.");
}

#[derive(Serialize)]
struct DeleteAgentResult<'a> {
    uid: &'a str,
    deleted: bool,
}

fn print_delete_result(uid: &str, output_format: OutputFormat) -> anyhow::Result<()> {
    match output_format {
        OutputFormat::Pretty => {
            println!("Deleted agent {uid}.");
        }
        OutputFormat::Text => {
            let mut stdout = std::io::stdout();
            writeln!(stdout, "{uid}")?;
        }
        OutputFormat::Ndjson => {
            super::output::write_json_line(
                &DeleteAgentResult { uid, deleted: true },
                std::io::stdout(),
            )?;
        }
        OutputFormat::Json => {
            super::output::write_json(
                &DeleteAgentResult { uid, deleted: true },
                std::io::stdout(),
            )?;
        }
    }
    Ok(())
}

fn display_optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn display_list<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let values = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

impl warpui::Entity for AgentManagementRunner {
    type Event = ();
}
impl SingletonEntity for AgentManagementRunner {}

#[cfg(test)]
#[path = "agent_management_tests.rs"]
mod tests;
