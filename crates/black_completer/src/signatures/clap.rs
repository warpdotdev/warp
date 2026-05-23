use clap::{Arg, Command as ClapCommand};
use warp_command_signatures::{
    Argument, IsArgumentOptional, Opt, ParserDirectives, Priority, Signature,
};

/// Convert a [`clap::Command`] into a [`Signature`]. All subcommands, options, and arguments are
/// preserved on a best-effort basis.
pub fn signature_from_clap_command(cmd: &mut ClapCommand, bin_name: &str) -> Signature {
    cmd.set_bin_name(bin_name);
    // Building the command sets all sorts of derived properties like Args::get_num_args.
    cmd.build();
    convert_command(cmd, cmd.get_bin_name().expect("Set above").to_string())
}

fn convert_command(cmd: &ClapCommand, name: String) -> Signature {
    let description = cmd.get_about().map(|s| s.to_string());

    let arguments = convert_positional_args(cmd);
    let options = convert_options(cmd);
    let subcommands = convert_subcommands(cmd);

    Signature {
        name,
        alias_generator: None,
        description,
        arguments,
        subcommands,
        options,
        priority: Priority::default(),
        parser_directives: ParserDirectives::default(),
    }
}

/// Convert clap positional arguments to Signature arguments.
fn convert_positional_args(cmd: &ClapCommand) -> Option<Vec<Argument>> {
    let positional_args: Vec<Argument> = cmd
        .get_positionals()
        .filter(|arg| !arg.is_hide_set())
        .map(convert_arg_to_argument)
        .collect();

    if positional_args.is_empty() {
        None
    } else {
        Some(positional_args)
    }
}

/// Convert clap options/flags to Signature options.
fn convert_options(cmd: &ClapCommand) -> Option<Vec<Opt>> {
    let opts: Vec<Opt> = cmd
        .get_opts()
        .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
        .map(convert_arg_to_opt)
        .collect();

    if opts.is_empty() {
        None
    } else {
        Some(opts)
    }
}

/// Convert clap subcommands to Signature subcommands by recursively converting each subcommand.
fn convert_subcommands(cmd: &ClapCommand) -> Option<Vec<Signature>> {
    let subcommands: Vec<Signature> = cmd
        .get_subcommands()
        .filter(|subcmd| !subcmd.is_hide_set())
        .flat_map(|cmd| {
            std::iter::once(cmd.get_name())
                .chain(cmd.get_visible_aliases())
                .map(|cmd_or_alias| convert_command(cmd, cmd_or_alias.to_string()))
                .collect::<Vec<_>>()
        })
        .collect();

    if subcommands.is_empty() {
        None
    } else {
        Some(subcommands)
    }
}

/// Convert a [`clap::Arg`] to a positional [`Argument`].
fn convert_arg_to_argument(arg: &Arg) -> Argument {
    let display_name = Some(arg.get_id().to_string());
    let description = arg.get_help().map(|s| s.to_string());
    let optional = if arg.is_required_set() {
        IsArgumentOptional::Required
    } else {
        IsArgumentOptional::Optional(
            arg.get_default_values()
                .first()
                .map(|s| s.to_string_lossy().to_string()),
        )
    };

    Argument {
        display_name,
        description,
        is_variadic: arg.get_num_args().is_some_and(|num_args| {
            num_args.takes_values() && num_args.min_values() != num_args.max_values()
        }),
        // TODO: Extract argument types from clap value hints.
        argument_types: vec![],
        optional,
        is_command: false,
        skip_generator_validation: true,
    }
}

/// Convert a [`clap::Arg]` to an [`Opt`] representing a flag/option.
fn convert_arg_to_opt(arg: &Arg) -> Opt {
    let mut exact_string = Vec::new();

    // Add short flags (e.g., "-h")
    for short in arg.get_short_and_visible_aliases().into_iter().flatten() {
        exact_string.push(format!("-{short}"));
    }

    // Add long flags (e.g., "--help")
    for long in arg.get_long_and_visible_aliases().into_iter().flatten() {
        exact_string.push(format!("--{long}"));
    }

    let description = arg.get_help().map(|s| s.to_string());
    let required = arg.is_required_set();

    let arguments = arg.get_num_args().and_then(|num_args| {
        if num_args.takes_values() {
            // TODO: Handle multi-valued flags. The Clap documentation is fairly unclear on how
            // it models this (e.g. can we assume that get_default_values and get_value names are
            // paired? Why is there only one ValueHint?). We currently don't need support for this.

            Some(vec![Argument {
                display_name: arg
                    .get_value_names()
                    .and_then(|names| names.first())
                    .map(|s| s.to_string()),
                description: None,
                is_variadic: false,
                // TODO: Infer from ValuesHint.
                argument_types: vec![],
                optional: arg
                    .get_default_values()
                    .first()
                    .map(|s| IsArgumentOptional::Optional(Some(s.to_string_lossy().to_string())))
                    .unwrap_or(IsArgumentOptional::Required),
                is_command: false,
                skip_generator_validation: true,
            }])
        } else {
            None
        }
    });

    Opt {
        exact_string,
        description,
        arguments,
        required,
        priority: Priority::default(),
    }
}
